use crate::index::Index;
use crate::parser::{byte_range_to_lsp, context, format_document, offset, offset_to_position};
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

impl Backend {
    async fn validate(&self, uri: Url, text: &str) {
        if let Some(index) = &self.index {
            let diags = index.diagnostics(text);
            self.client.publish_diagnostics(uri, diags, None).await;
        }
    }
}

#[tower_lsp::async_trait]
impl tower_lsp::LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                document_formatting_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![
                        "<".into(),
                        " ".into(),
                        ".".into(),
                        "/".into(),
                        ":".into(),
                        "\"".into(),
                        "{".into(),
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

    async fn formatting(&self, p: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let docs = self.docs.lock().unwrap();
        let Some(text) = docs.get(&p.text_document.uri) else {
            return Ok(None);
        };

        let tab_size = p.options.tab_size as usize;
        let insert_spaces = p.options.insert_spaces;
        let formatted = format_document(text, tab_size, insert_spaces);

        if &formatted == text {
            return Ok(None);
        }

        let full_range = byte_range_to_lsp(text, 0, text.len());
        Ok(Some(vec![TextEdit {
            range: full_range,
            new_text: formatted,
        }]))
    }

    async fn did_open(&self, p: DidOpenTextDocumentParams) {
        let uri = p.text_document.uri;
        let text = p.text_document.text;
        self.docs.lock().unwrap().insert(uri.clone(), text.clone());
        self.validate(uri, &text).await;
    }

    async fn did_change(&self, p: DidChangeTextDocumentParams) {
        if let Some(c) = p.content_changes.into_iter().last() {
            let uri = p.text_document.uri;
            let text = c.text;
            self.docs.lock().unwrap().insert(uri.clone(), text.clone());
            self.validate(uri, &text).await;
        }
    }

    async fn did_close(&self, p: DidCloseTextDocumentParams) {
        self.docs.lock().unwrap().remove(&p.text_document.uri);
        self.client
            .publish_diagnostics(p.text_document.uri, vec![], None)
            .await;
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

    async fn goto_definition(
        &self,
        p: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let Some(index) = &self.index else {
            return Ok(None);
        };
        let uri = p.text_document_position_params.text_document.uri;
        let pos = p.text_document_position_params.position;

        let ev = {
            let docs = self.docs.lock().unwrap();
            let Some(text) = docs.get(&uri) else {
                return Ok(None);
            };
            let Some(off) = offset(text, pos) else {
                return Ok(None);
            };
            index.event_at_offset(text, off)
        };

        let Some(ev) = ev else {
            return Ok(None);
        };

        let Ok(axaml_path) = uri.to_file_path() else {
            return Ok(None);
        };
        let cs_path = axaml_path.with_extension("axaml.cs");
        let Ok(cs_text) = std::fs::read_to_string(&cs_path) else {
            return Ok(None);
        };
        let Ok(cs_uri) = Url::from_file_path(&cs_path) else {
            return Ok(None);
        };

        let needle = format!("void {}", ev.handler_name);
        if let Some(idx) = cs_text.find(&needle) {
            let m_start = idx + 5;
            let m_end = m_start + ev.handler_name.len();
            return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                uri: cs_uri,
                range: byte_range_to_lsp(&cs_text, m_start, m_end),
            })));
        }

        Ok(None)
    }

    async fn code_action(&self, p: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let Some(index) = &self.index else {
            return Ok(None);
        };
        let uri = p.text_document.uri;
        let pos = p.range.start;

        let ev = {
            let docs = self.docs.lock().unwrap();
            let Some(text) = docs.get(&uri) else {
                return Ok(None);
            };
            let Some(off) = offset(text, pos) else {
                return Ok(None);
            };
            index.event_at_offset(text, off)
        };

        let Some(ev) = ev else {
            return Ok(None);
        };

        let Ok(axaml_path) = uri.to_file_path() else {
            return Ok(None);
        };
        let cs_path = axaml_path.with_extension("axaml.cs");
        let Ok(cs_text) = std::fs::read_to_string(&cs_path) else {
            return Ok(None);
        };
        if cs_text.contains(&format!("void {}", ev.handler_name)) {
            return Ok(None);
        }

        let Some(insert_off) = find_class_insert_offset(&cs_text) else {
            return Ok(None);
        };
        let Ok(cs_uri) = Url::from_file_path(&cs_path) else {
            return Ok(None);
        };

        let args_type = event_args_type(&ev.event_name);
        let insert_pos = offset_to_position(&cs_text, insert_off);
        let method_stub = format!(
            "\n    private void {}(object? sender, {} e)\n    {{\n    }}\n",
            ev.handler_name, args_type
        );

        let mut changes = HashMap::new();
        changes.insert(
            cs_uri,
            vec![TextEdit {
                range: Range {
                    start: insert_pos,
                    end: insert_pos,
                },
                new_text: method_stub,
            }],
        );

        let action = CodeAction {
            title: format!(
                "Создать обработчик '{}' в .axaml.cs ({})",
                ev.handler_name, ev.tag_name
            ),
            kind: Some(CodeActionKind::QUICKFIX),
            edit: Some(WorkspaceEdit {
                changes: Some(changes),
                ..Default::default()
            }),
            is_preferred: Some(true),
            ..Default::default()
        };

        Ok(Some(vec![CodeActionOrCommand::CodeAction(action)]))
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

fn event_args_type(event_name: &str) -> &'static str {
    match event_name {
        "TextChanged" => "Avalonia.Controls.TextChangedEventArgs",
        "SelectionChanged" | "SelectedDateChanged" => "Avalonia.Controls.SelectionChangedEventArgs",
        "KeyDown" | "KeyUp" => "Avalonia.Input.KeyEventArgs",
        "PointerPressed" => "Avalonia.Input.PointerPressedEventArgs",
        "PointerReleased" => "Avalonia.Input.PointerReleasedEventArgs",
        "PointerMoved" => "Avalonia.Input.PointerEventArgs",
        "Closing" => "Avalonia.Controls.WindowClosingEventArgs",
        _ => "Avalonia.Interactivity.RoutedEventArgs",
    }
}

/// Находит позицию закрывающей скобки `}` класса в `.axaml.cs`
fn find_class_insert_offset(cs_text: &str) -> Option<usize> {
    let class_pos = cs_text.find("class ")?;
    let open_brace = cs_text[class_pos..].find('{')? + class_pos;
    let mut depth = 0i32;
    for (i, ch) in cs_text[open_brace..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open_brace + i);
                }
            }
            _ => {}
        }
    }
    None
}
