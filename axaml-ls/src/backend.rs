use crate::actions;
use crate::index::Index;
use crate::parser::{byte_range_to_lsp, context, format_document, offset, Ctx};
use crate::workspace::{parse_xmlns_map, resolve_xml_type, WorkspaceData};
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
            let mut diags = index.diagnostics(text);
            let handler_diags = {
                let docs = self.docs.lock().unwrap();
                actions::missing_handler_diagnostics(index, &docs, &uri, text)
            };
            crate::workspace::sync_named_controls_g_cs(&uri, text);
            let ws = WorkspaceData::scan_for_file(&uri, text);
            diags.extend(handler_diags);
            diags.extend(ws.binding_diagnostics(text));
            self.client.publish_diagnostics(uri, diags, None).await;
        }
    }
}

#[tower_lsp::async_trait]
impl tower_lsp::LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec![actions::CREATE_HANDLER.to_string(), actions::CREATE_VIEW.to_string(), actions::INIT_VIEW_CS.to_string()],
                    ..Default::default()
                }),
                document_formatting_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
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
            "axaml-ls: metadata loaded"
        } else {
            "axaml-ls: meta.json not found"
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

        let (text, off, ctx, extra) = {
            let docs = self.docs.lock().unwrap();
            let Some(text) = docs.get(&uri).cloned() else {
                return Ok(None);
            };
            let Some(off) = offset(&text, pos) else {
                return Ok(None);
            };
            let ctx = context(&text, off, pos);
            let extra = actions::handler_completion(index, &docs, &uri, &text, off);
            (text, off, ctx, extra)
        };

        let mut ws_items = Vec::new();
        match &ctx {
            Ctx::AttrValue { prefix, replace, .. } => {
                let ws = WorkspaceData::scan_for_file(&uri, &text);
                ws_items.extend(ws.complete_attr_value(&text, off, prefix, *replace));
            }
            Ctx::Attr { tag, replace, .. } if tag.contains(':') => {
                let ws = WorkspaceData::scan_for_file(&uri, &text);
                let xmlns = parse_xmlns_map(&text);
                let (ns, cls_name) = resolve_xml_type(tag, &xmlns);
                if ws.is_control_class(&ns, &cls_name) {
                    if let Some(cls) = ws.find_class(&ns, &cls_name) {
                        for m in &cls.members {
                            ws_items.push(CompletionItem {
                                label: m.name.clone(),
                                sort_text: Some(format!("1_{}", m.name)),
                                kind: Some(CompletionItemKind::PROPERTY),
                                detail: Some(format!("{cls_name}.{} ({})", m.name, m.member_type)),
                                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                                    range: *replace,
                                    new_text: format!("{}=\"$0\"", m.name),
                                })),
                                insert_text_format: Some(InsertTextFormat::SNIPPET),
                                ..Default::default()
                            });
                        }
                    }
                }
            }
            Ctx::Tag { replace, has_close_bracket, .. } => {
                let ws = WorkspaceData::scan_for_file(&uri, &text);
                let xmlns = parse_xmlns_map(&text);
                for (prefix, ns) in &xmlns {
                    if matches!(prefix.as_str(), "x" | "d" | "mc") {
                        continue;
                    }
                    for cls in &ws.classes {
                        if cls.namespace == *ns && ws.is_control_class(ns, &cls.name) {
                            let label = format!("{prefix}:{}", cls.name);
                            let new_text = if *has_close_bracket {
                                label.clone()
                            } else {
                                format!("{label}>$0</{label}>")
                            };
                            ws_items.push(CompletionItem {
                                label: label.clone(),
                                sort_text: Some(format!("1_{label}")),
                                kind: Some(CompletionItemKind::CLASS),
                                detail: Some(format!("{}.{}", cls.namespace, cls.name)),
                                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                                    range: *replace,
                                    new_text,
                                })),
                                insert_text_format: Some(InsertTextFormat::SNIPPET),
                                ..Default::default()
                            });
                        }
                    }
                }
            }
            _ => {}
        }

        let mut items = index.complete(ctx);
        items.extend(actions::init_empty_file_completions(&uri, &text));
        items.extend(ws_items);
        if let Some(item) = extra {
            items.insert(0, item);
        }
        Ok(Some(CompletionResponse::Array(items)))
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

        let (text, off, ev, class_loc) = {
            let docs = self.docs.lock().unwrap();
            let Some(text) = docs.get(&uri).cloned() else {
                return Ok(None);
            };
            let Some(off) = offset(&text, pos) else {
                return Ok(None);
            };
            (
                text.clone(),
                off,
                index.event_at_offset(&text, off),
                actions::goto_x_class(&docs, &uri, &text, off),
            )
        };

        if let Some(loc) = class_loc {
            return Ok(Some(GotoDefinitionResponse::Scalar(loc)));
        }

        let ws = WorkspaceData::scan_for_file(&uri, &text);
        if let Some(res_loc) = ws.goto_resource(&text, off) {
            return Ok(Some(GotoDefinitionResponse::Scalar(res_loc)));
        }
        if let Some(type_loc) = ws.goto_type(&text, off) {
            return Ok(Some(GotoDefinitionResponse::Scalar(type_loc)));
        }

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

        let list = {
            let docs = self.docs.lock().unwrap();
            let Some(text) = docs.get(&uri) else {
                return Ok(None);
            };
            let Some(off) = offset(text, pos) else {
                return Ok(None);
            };
            let mut res = Vec::new();
            if let Some(a) = actions::create_handler(index, &docs, &uri, text, off) {
                res.push(CodeActionOrCommand::CodeAction(a));
            }
            for a in actions::init_empty_file_actions(&uri, text) {
                res.push(CodeActionOrCommand::CodeAction(a));
            }
            for a in actions::create_view_actions(&uri, text, off) {
                res.push(CodeActionOrCommand::CodeAction(a));
            }
            res
        };

        if list.is_empty() {
            Ok(None)
        } else {
            Ok(Some(list))
        }
    }

    async fn execute_command(&self, p: ExecuteCommandParams) -> Result<Option<serde_json::Value>> {
        let arg = |i: usize| {
            p.arguments
                .get(i)
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };


        if p.command == actions::INIT_VIEW_CS {
            if let (Some(uri_s), Some(kind), Some(name)) = (arg(0), arg(1), arg(2)) {
                if let Ok(uri) = Url::parse(&uri_s) {
                    actions::run_init_view_cs(&uri, &kind, &name);
                }
            }
            return Ok(None);
        }

        if p.command == actions::CREATE_VIEW {
            if let (Some(uri_s), Some(kind), Some(name)) = (arg(0), arg(1), arg(2)) {
                if let Ok(uri) = Url::parse(&uri_s) {
                    if let Some(created) = actions::run_create_view(&uri, &kind, &name) {
                        if let Ok(target_uri) = Url::from_file_path(&created) {
                            let _ = self
                                .client
                                .show_document(ShowDocumentParams {
                                    uri: target_uri,
                                    external: Some(false),
                                    take_focus: Some(true),
                                    selection: None,
                                })
                                .await;
                        }
                    }
                }
            }
            return Ok(None);
        }

        if p.command != actions::CREATE_HANDLER {
            return Ok(None);
        }
        let Some(index) = &self.index else {
            return Ok(None);
        };
        let arg = |i: usize| {
            p.arguments
                .get(i)
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        let (Some(uri), Some(tag), Some(event), Some(handler)) = (arg(0), arg(1), arg(2), arg(3))
        else {
            return Ok(None);
        };
        let Ok(uri) = Url::parse(&uri) else {
            return Ok(None);
        };

        let result = {
            let docs = self.docs.lock().unwrap();
            actions::run_create_handler(index, &docs, &uri, &tag, &event, &handler)
        };
        let Some((edit, path, new_text)) = result else {
            return Ok(None);
        };

        let applied = match self.client.apply_edit(edit).await {
            Ok(r) => r.applied,
            Err(_) => false,
        };
        if !applied {
            let _ = std::fs::write(&path, new_text);
        }
        Ok(None)
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
