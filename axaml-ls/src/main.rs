use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

#[derive(Deserialize)]
struct TypeMeta {
    name: String,
    #[allow(dead_code)]
    namespace: String,
    base: Option<String>,
    properties: Vec<String>,
    attached: Vec<String>,
}

struct Index {
    types: HashMap<String, TypeMeta>,
    /// (простое имя, полное имя) для типов, которые можно писать тегом
    tags: Vec<(String, String)>,
}

enum Ctx {
    None,
    Tag,
    Attr(String),
}

impl Index {
    fn load() -> Option<Index> {
        let path = std::env::var("AXAML_META").ok().or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| format!("{h}/.local/share/axaml-ls/meta.json"))
        })?;
        let data = std::fs::read_to_string(path).ok()?;
        let types: HashMap<String, TypeMeta> = serde_json::from_str(&data).ok()?;

        let mut tags: Vec<(String, String)> = types
            .iter()
            .filter(|(full, t)| {
                !t.name.contains('`') && Self::inherits(&types, full, "Avalonia.Controls.Control")
            })
            .map(|(full, t)| (t.name.clone(), full.clone()))
            .collect();
        tags.sort();

        Some(Index { types, tags })
    }

    fn inherits(types: &HashMap<String, TypeMeta>, start: &str, target: &str) -> bool {
        let mut cur = Some(start.to_string());
        for _ in 0..64 {
            let Some(n) = cur else { return false };
            if n == target {
                return true;
            }
            cur = types.get(&n).and_then(|t| t.base.clone());
        }
        false
    }

    /// сам тип и все его базовые, от потомка к предку
    fn chain(&self, full: &str) -> Vec<&TypeMeta> {
        let mut out = Vec::new();
        let mut cur = Some(full.to_string());
        for _ in 0..64 {
            let Some(n) = cur else { break };
            match self.types.get(&n) {
                Some(t) => {
                    out.push(t);
                    cur = t.base.clone();
                }
                None => break,
            }
        }
        out
    }

    fn complete(&self, ctx: Ctx) -> Vec<CompletionItem> {
        match ctx {
            Ctx::None => vec![],
            Ctx::Tag => self
                .tags
                .iter()
                .map(|(name, full)| CompletionItem {
                    label: name.clone(),
                    kind: Some(CompletionItemKind::CLASS),
                    detail: Some(full.clone()),
                    ..Default::default()
                })
                .collect(),
            Ctx::Attr(tag) => {
                let Some((_, full)) = self.tags.iter().find(|(n, _)| *n == tag) else {
                    return vec![];
                };
                let mut seen = HashSet::new();
                let mut items = Vec::new();

                for t in self.chain(full) {
                    for p in &t.properties {
                        // "Item" это индексатор, в XAML он не нужен
                        if p == "Item" || !seen.insert(p.clone()) {
                            continue;
                        }
                        items.push(attr_item(p.clone(), &t.name));
                    }
                }
                // прикреплённые свойства: Grid.Row, ToolTip.Tip и т.п.
                for t in self.types.values() {
                    for a in &t.attached {
                        items.push(attr_item(format!("{}.{}", t.name, a), "attached"));
                    }
                }
                items
            }
        }
    }
}

fn attr_item(label: String, detail: &str) -> CompletionItem {
    CompletionItem {
        insert_text: Some(format!("{label}=\"$0\"")),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        label,
        kind: Some(CompletionItemKind::PROPERTY),
        detail: Some(detail.to_string()),
        ..Default::default()
    }
}

/// позиция LSP (строка, символ в UTF-16) в байтовый сдвиг
fn offset(text: &str, pos: Position) -> Option<usize> {
    let mut line_start = 0;
    for (i, line) in text.split('\n').enumerate() {
        if i as u32 == pos.line {
            let mut units = 0u32;
            for (bi, ch) in line.char_indices() {
                if units >= pos.character {
                    return Some(line_start + bi);
                }
                units += ch.len_utf16() as u32;
            }
            return Some(line_start + line.len());
        }
        line_start += line.len() + 1;
    }
    None
}

/// где стоит курсор: в имени тега, в атрибутах или вне тега
fn context(before: &str) -> Ctx {
    let Some(lt) = before.rfind('<') else {
        return Ctx::None;
    };
    if let Some(gt) = before.rfind('>') {
        if gt > lt {
            return Ctx::None;
        }
    }
    let seg = &before[lt + 1..];
    if seg.starts_with(['/', '!', '?']) {
        return Ctx::None;
    }
    match seg.find(char::is_whitespace) {
        None => Ctx::Tag,
        Some(ws) => {
            // нечётное число кавычек: мы внутри значения атрибута
            if seg.matches('"').count() % 2 == 1 {
                return Ctx::None;
            }
            Ctx::Attr(seg[..ws].to_string())
        }
    }
}

struct Backend {
    client: Client,
    index: Option<Index>,
    docs: Mutex<HashMap<Url, String>>,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec!["<".into(), " ".into(), ".".into()]),
                    ..Default::default()
                }),
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
            self.docs.lock().unwrap().insert(p.text_document.uri, c.text);
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
            context(&text[..off])
        };

        Ok(Some(CompletionResponse::Array(index.complete(ctx))))
    }
}

#[tokio::main]
async fn main() {
    let index = Index::load();
    let (stdin, stdout) = (tokio::io::stdin(), tokio::io::stdout());
    let (service, socket) = LspService::new(|client| Backend {
        client,
        index,
        docs: Mutex::new(HashMap::new()),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}
