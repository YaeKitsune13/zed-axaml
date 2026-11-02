use crate::index::Index;
use crate::parser::{context, offset};
use std::collections::HashMap;
use std::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::Client;

pub struct Backend {
    pub client: Client,
    pub index: Option<Index>,
    pub docs: Mutex<HashMap<Url, String>>,
}

#[tower_lsp::async_trait]
impl tower_lsp::LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![
                        "<".into(),
                        " ".into(),
                        ".".into(),
                        "/".into(),
                        ":".into(),
                    ]),
                    ..Default::default()
                }),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            work_done_progress_options: Default::default(),
                            legend: SemanticTokensLegend {
                                token_types: vec![
                                    SemanticTokenType::PROPERTY,
                                    SemanticTokenType::EVENT,
                                ],
                                token_modifiers: vec![],
                            },
                            range: None,
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                        },
                    ),
                ),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        let msg = if self.index.is_some() {
            "axaml-ls: метаданные загружены"
        } else {
            "axaml-ls: meta.json не найден"
        };
        self.client.log_message(MessageType::INFO, msg).await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, p: DidOpenTextDocumentParams) {
        self.docs
            .lock()
            .unwrap()
            .insert(p.text_document.uri, p.text_document.text);
    }

    async fn did_change(&self, p: DidChangeTextDocumentParams) {
        if let Some(c) = p.content_changes.into_iter().last() {
            self.docs
                .lock()
                .unwrap()
                .insert(p.text_document.uri, c.text);
        }
    }

    async fn did_close(&self, p: DidCloseTextDocumentParams) {
        self.docs.lock().unwrap().remove(&p.text_document.uri);
    }

    async fn completion(&self, p: CompletionParams) -> Result<Option<CompletionResponse>> {
        let Some(index) = &self.index else {
            return Ok(None);
        };
        let uri = p.text_document_position.text_document.uri;
        let pos = p.text_document_position.position;

        let ctx = {
            let docs = self.docs.lock().unwrap();
            let Some(text) = docs.get(&uri) else {
                return Ok(None);
            };
            let Some(off) = offset(text, pos) else {
                return Ok(None);
            };
            context(text, off, pos)
        };

        Ok(Some(CompletionResponse::Array(index.complete(ctx))))
    }

    async fn hover(&self, p: HoverParams) -> Result<Option<Hover>> {
        let Some(index) = &self.index else {
            return Ok(None);
        };
        let uri = p.text_document_position_params.text_document.uri;
        let pos = p.text_document_position_params.position;

        let docs = self.docs.lock().unwrap();
        let Some(text) = docs.get(&uri) else {
            return Ok(None);
        };
        let Some(off) = offset(text, pos) else {
            return Ok(None);
        };
        Ok(index.hover(text, off))
    }

    async fn semantic_tokens_full(
        &self,
        p: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let Some(index) = &self.index else {
            return Ok(None);
        };
        let data = {
            let docs = self.docs.lock().unwrap();
            let Some(text) = docs.get(&p.text_document.uri) else {
                return Ok(None);
            };
            index.semantic_tokens(text)
        };
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data,
        })))
    }
}
