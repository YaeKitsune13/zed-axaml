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
    #[serde(default)]
    events: Vec<String>,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    docs: HashMap<String, String>,
}

struct Index {
    types: HashMap<String, TypeMeta>,
    /// (простое имя, полное имя) для типов, которые можно писать тегом
    tags: Vec<(String, String)>,
}

enum Ctx {
    None,
    Tag,
    Close(String),
    Attr(String, String), // (тег, набранный префикс)
}

// ---------- разбор XML-тегов ----------

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Open,
    Close,
    SelfClose,
}

struct Tag {
    kind: Kind,
    name: String,
    /// (имя атрибута, байтовый сдвиг начала имени в тексте)
    attrs: Vec<(String, usize)>,
}

fn scan(text: &str) -> Vec<Tag> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;

    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        if text[i..].starts_with("<!--") {
            match text[i..].find("-->") {
                Some(e) => {
                    i += e + 3;
                    continue;
                }
                None => break,
            }
        }

        // конец тега, пропуская кавычки
        let mut j = i + 1;
        let mut quote: Option<u8> = None;
        while j < b.len() {
            match quote {
                Some(q) => {
                    if b[j] == q {
                        quote = None;
                    }
                }
                None => match b[j] {
                    b'"' | b'\'' => quote = Some(b[j]),
                    b'>' => break,
                    _ => {}
                },
            }
            j += 1;
        }
        if j >= b.len() {
            break; // тег не закрыт (курсор внутри него)
        }

        let inner_start = i + 1;
        let inner = &text[inner_start..j];
        i = j + 1;

        if inner.starts_with('?') || inner.starts_with('!') {
            continue;
        }
        if let Some(rest) = inner.strip_prefix('/') {
            out.push(Tag {
                kind: Kind::Close,
                name: rest.trim().to_string(),
                attrs: vec![],
            });
            continue;
        }

        let self_close = inner.trim_end().ends_with('/');
        let name_end = inner
            .find(|c: char| c.is_whitespace())
            .unwrap_or(inner.len());
        let name = inner[..name_end].trim_end_matches('/').to_string();

        let ib = inner.as_bytes();
        let mut attrs = Vec::new();
        let mut k = name_end;
        while k < ib.len() {
            if ib[k].is_ascii_whitespace() {
                k += 1;
                continue;
            }
            let s = k;
            while k < ib.len() && !ib[k].is_ascii_whitespace() && ib[k] != b'=' && ib[k] != b'/' {
                k += 1;
            }
            if k > s {
                attrs.push((inner[s..k].to_string(), inner_start + s));
            }
            while k < ib.len() && ib[k].is_ascii_whitespace() {
                k += 1;
            }
            if k < ib.len() && ib[k] == b'=' {
                k += 1;
                while k < ib.len() && ib[k].is_ascii_whitespace() {
                    k += 1;
                }
                if k < ib.len() && (ib[k] == b'"' || ib[k] == b'\'') {
                    let q = ib[k];
                    k += 1;
                    while k < ib.len() && ib[k] != q {
                        k += 1;
                    }
                    k += 1;
                } else {
                    while k < ib.len() && !ib[k].is_ascii_whitespace() {
                        k += 1;
                    }
                }
            } else if k == s {
                k += 1; // например "/" перед ">"
            }
        }

        out.push(Tag {
            kind: if self_close {
                Kind::SelfClose
            } else {
                Kind::Open
            },
            name,
            attrs,
        });
    }
    out
}

/// стек незакрытых тегов в тексте
fn open_stack(text: &str) -> Vec<String> {
    let mut stack: Vec<String> = Vec::new();
    for tag in scan(text) {
        match tag.kind {
            Kind::Open => stack.push(tag.name),
            Kind::SelfClose => {}
            Kind::Close => {
                if let Some(p) = stack.iter().rposition(|n| *n == tag.name) {
                    stack.truncate(p);
                }
            }
        }
    }
    stack
}

// ---------- индекс ----------

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

    fn is_attached(&self, attr: &str) -> bool {
        let Some((owner, name)) = attr.split_once('.') else {
            return false;
        };
        self.types
            .values()
            .any(|t| t.name == owner && t.attached.iter().any(|a| a == name))
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
                    documentation: self
                        .types
                        .get(full)
                        .filter(|t| !t.summary.is_empty())
                        .map(|t| Documentation::String(t.summary.clone())),
                    insert_text: Some(format!("{name}>$0</{name}>")),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    ..Default::default()
                })
                .collect(),
            Ctx::Close(name) => vec![CompletionItem {
                insert_text: Some(format!("{name}>")),
                label: name,
                kind: Some(CompletionItemKind::CLASS),
                detail: Some("закрывающий тег".to_string()),
                ..Default::default()
            }],
            Ctx::Attr(tag, prefix) => {
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
                        items.push(attr_item(
                            p.clone(),
                            &t.name,
                            CompletionItemKind::PROPERTY,
                            t.docs.get(&format!("P:{p}")),
                        ));
                    }
                    for e in &t.events {
                        if !seen.insert(e.clone()) {
                            continue;
                        }
                        items.push(attr_item(
                            e.clone(),
                            &t.name,
                            CompletionItemKind::EVENT,
                            t.docs.get(&format!("E:{e}")),
                        ));
                    }
                }

                // прикреплённые свойства (Grid.Row и т.п.) только когда набрана точка
                if prefix.contains('.') {
                    for t in self.types.values() {
                        for a in &t.attached {
                            items.push(attr_item(
                                format!("{}.{}", t.name, a),
                                "attached",
                                CompletionItemKind::PROPERTY,
                                t.docs.get(&format!("A:{a}")),
                            ));
                        }
                    }
                }
                items
            }
        }
    }

    /// семантические токены: 0 = свойство, 1 = событие
    fn semantic_tokens(&self, text: &str) -> Vec<SemanticToken> {
        let mut line_starts = vec![0usize];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }

        let mut raw: Vec<(u32, u32, u32, u32)> = Vec::new(); // строка, колонка, длина, тип
        for tag in scan(text) {
            if tag.kind == Kind::Close {
                continue;
            }
            let Some((_, full)) = self.tags.iter().find(|(n, _)| *n == tag.name) else {
                continue;
            };
            let chain = self.chain(full);

            for (attr, off) in &tag.attrs {
                let ty = if chain.iter().any(|t| t.events.contains(attr)) {
                    1
                } else if chain.iter().any(|t| t.properties.contains(attr))
                    || self.is_attached(attr)
                {
                    0
                } else {
                    continue;
                };

                let line = line_starts.partition_point(|&s| s <= *off) - 1;
                let col = text[line_starts[line]..*off].encode_utf16().count() as u32;
                let len = attr.encode_utf16().count() as u32;
                raw.push((line as u32, col, len, ty));
            }
        }

        let (mut pl, mut pc) = (0u32, 0u32);
        raw.into_iter()
            .map(|(l, c, len, ty)| {
                let dl = l - pl;
                let ds = if dl == 0 { c - pc } else { c };
                pl = l;
                pc = c;
                SemanticToken {
                    delta_line: dl,
                    delta_start: ds,
                    length: len,
                    token_type: ty,
                    token_modifiers_bitset: 0,
                }
            })
            .collect()
    }
}

fn attr_item(
    label: String,
    detail: &str,
    kind: CompletionItemKind,
    doc: Option<&String>,
) -> CompletionItem {
    CompletionItem {
        insert_text: Some(format!("{label}=\"$0\"")),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        documentation: doc
            .filter(|d| !d.is_empty())
            .map(|d| Documentation::String(d.clone())),
        label,
        kind: Some(kind),
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

/// где стоит курсор: в имени тега, в закрывающем теге, в атрибутах или вне тега
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

    if let Some(rest) = seg.strip_prefix('/') {
        if rest.contains(char::is_whitespace) {
            return Ctx::None;
        }
        return match open_stack(&before[..lt]).pop() {
            Some(name) => Ctx::Close(name),
            None => Ctx::None,
        };
    }
    if seg.starts_with(['!', '?']) {
        return Ctx::None;
    }

    match seg.find(char::is_whitespace) {
        None => Ctx::Tag,
        Some(ws) => {
            // нечётное число кавычек: мы внутри значения атрибута
            if seg.matches('"').count() % 2 == 1 {
                return Ctx::None;
            }
            let prefix: String = seg
                .chars()
                .rev()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            Ctx::Attr(seg[..ws].to_string(), prefix)
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
                    trigger_characters: Some(vec!["<".into(), " ".into(), ".".into(), "/".into()]),
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
            context(&text[..off])
        };

        Ok(Some(CompletionResponse::Array(index.complete(ctx))))
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
