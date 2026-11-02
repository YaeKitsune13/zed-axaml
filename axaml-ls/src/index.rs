use crate::metadata::{TypeMeta, TAG_BASES, X_DIRECTIVES};
use crate::parser::{inside_quotes, is_ident_char, scan, Ctx, Kind};
use std::collections::{HashMap, HashSet};
use tower_lsp::lsp_types::*;

pub struct Index {
    pub types: HashMap<String, TypeMeta>,
    pub tags: Vec<(String, String)>,
}

impl Index {
    pub fn load() -> Option<Index> {
        let path = std::env::var("AXAML_META").ok().or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| format!("{h}/.local/share/axaml-ls/meta.json"))
        })?;
        let data = std::fs::read_to_string(path).ok()?;
        let types: HashMap<String, TypeMeta> = serde_json::from_str(&data).ok()?;

        let mut seen_names = HashSet::new();
        let mut tags: Vec<(String, String)> = types
            .iter()
            .filter(|(full, t)| {
                !t.name.contains('`')
                    && TAG_BASES
                        .iter()
                        .any(|base| Self::inherits(&types, full, base))
            })
            .map(|(full, t)| (t.name.clone(), full.clone()))
            .collect();
        tags.sort();
        tags.retain(|(name, _)| seen_names.insert(name.clone()));

        Some(Index { types, tags })
    }

    pub fn inherits(types: &HashMap<String, TypeMeta>, start: &str, target: &str) -> bool {
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

    pub fn chain(&self, full: &str) -> Vec<&TypeMeta> {
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

    pub fn is_attached(&self, attr: &str) -> bool {
        let Some((owner, name)) = attr.split_once('.') else {
            return false;
        };
        self.types
            .values()
            .any(|t| t.name == owner && t.attached.iter().any(|a| a == name))
    }

    fn push_prop_tags(
        &self,
        owner: &str,
        replace: Range,
        has_close_bracket: bool,
        out: &mut Vec<CompletionItem>,
    ) {
        let Some((_, full)) = self.tags.iter().find(|(n, _)| n == owner) else {
            return;
        };
        let mut seen = HashSet::new();
        for t in self.chain(full) {
            for p in &t.properties {
                if p == "Item" || !seen.insert(p.clone()) {
                    continue;
                }
                let label = format!("{owner}.{p}");
                let new_text = if has_close_bracket {
                    label.clone()
                } else {
                    format!("{label}>$0</{label}>")
                };
                out.push(CompletionItem {
                    label,
                    kind: Some(CompletionItemKind::PROPERTY),
                    detail: Some(format!("{}.{}", t.name, p)),
                    documentation: t
                        .docs
                        .get(&format!("P:{p}"))
                        .filter(|d| !d.is_empty())
                        .map(|d| Documentation::String(d.clone())),
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                        range: replace,
                        new_text,
                    })),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    ..Default::default()
                });
            }
        }
    }

    pub fn complete(&self, ctx: Ctx) -> Vec<CompletionItem> {
        match ctx {
            Ctx::None => vec![],
            Ctx::Tag {
                prefix,
                parent,
                replace,
                has_close_bracket,
            } => {
                if let Some((owner, _)) = prefix.split_once('.') {
                    let mut items = Vec::new();
                    self.push_prop_tags(owner, replace, has_close_bracket, &mut items);
                    return items;
                }

                let mut items: Vec<CompletionItem> = self
                    .tags
                    .iter()
                    .map(|(name, full)| {
                        let new_text = if has_close_bracket {
                            name.clone()
                        } else {
                            format!("{name}>$0</{name}>")
                        };
                        CompletionItem {
                            label: name.clone(),
                            kind: Some(CompletionItemKind::CLASS),
                            detail: Some(full.clone()),
                            documentation: self
                                .types
                                .get(full)
                                .filter(|t| !t.summary.is_empty())
                                .map(|t| Documentation::String(t.summary.clone())),
                            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                                range: replace,
                                new_text,
                            })),
                            insert_text_format: Some(InsertTextFormat::SNIPPET),
                            ..Default::default()
                        }
                    })
                    .collect();

                if let Some(p) = parent {
                    let owner = p.split('.').next().unwrap_or(&p);
                    self.push_prop_tags(owner, replace, has_close_bracket, &mut items);
                }

                items
            }
            Ctx::Close {
                name,
                replace,
                has_close_bracket,
            } => {
                let new_text = if has_close_bracket {
                    name.clone()
                } else {
                    format!("{name}>")
                };
                vec![CompletionItem {
                    label: name,
                    kind: Some(CompletionItemKind::CLASS),
                    detail: Some("закрывающий тег".to_string()),
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                        range: replace,
                        new_text,
                    })),
                    ..Default::default()
                }]
            }
            Ctx::Attr {
                tag,
                prefix,
                replace,
            } => {
                if tag.contains('.') {
                    return vec![];
                }
                let Some((_, full)) = self.tags.iter().find(|(n, _)| *n == tag) else {
                    return vec![];
                };
                let mut seen = HashSet::new();
                let mut items = Vec::new();

                for t in self.chain(full) {
                    for p in &t.properties {
                        if p == "Item" || !seen.insert(p.clone()) {
                            continue;
                        }
                        items.push(attr_item(
                            p.clone(),
                            &t.name,
                            CompletionItemKind::PROPERTY,
                            t.docs.get(&format!("P:{p}")),
                            replace,
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
                            replace,
                        ));
                    }
                }

                for &(dir, desc) in X_DIRECTIVES {
                    let doc_str = desc.to_string();
                    items.push(attr_item(
                        dir.to_string(),
                        "XAML directive",
                        CompletionItemKind::KEYWORD,
                        Some(&doc_str),
                        replace,
                    ));
                }

                if prefix.contains('.') {
                    for t in self.types.values() {
                        for a in &t.attached {
                            items.push(attr_item(
                                format!("{}.{}", t.name, a),
                                "attached",
                                CompletionItemKind::PROPERTY,
                                t.docs.get(&format!("A:{a}")),
                                replace,
                            ));
                        }
                    }
                }
                items
            }
        }
    }

    pub fn find_member(&self, full: &str, member: &str) -> Option<(&str, &'static str, String)> {
        for t in self.chain(full) {
            if t.properties.iter().any(|p| p == member) {
                let doc = t
                    .docs
                    .get(&format!("P:{member}"))
                    .cloned()
                    .unwrap_or_default();
                return Some((&t.name, "property", doc));
            }
            if t.events.iter().any(|e| e == member) {
                let doc = t
                    .docs
                    .get(&format!("E:{member}"))
                    .cloned()
                    .unwrap_or_default();
                return Some((&t.name, "event", doc));
            }
        }
        None
    }

    pub fn hover(&self, text: &str, off: usize) -> Option<Hover> {
        let before = &text[..off];
        let lt = before.rfind('<')?;
        if before[lt..].contains('>') || inside_quotes(&before[lt + 1..]) {
            return None;
        }

        let start = before
            .char_indices()
            .rev()
            .take_while(|&(_, c)| is_ident_char(c))
            .last()
            .map(|(i, _)| i)
            .unwrap_or(off);
        let end = off
            + text[off..]
                .chars()
                .take_while(|&c| is_ident_char(c))
                .map(char::len_utf8)
                .sum::<usize>();

        if start == end {
            return None;
        }
        let word = &text[start..end];
        let before_word = text[lt + 1..start].trim_start_matches('/');

        let md = if before_word.is_empty() {
            if let Some((owner, prop)) = word.split_once('.') {
                let (_, full) = self.tags.iter().find(|(n, _)| n == owner)?;
                let (decl_owner, kind, doc) = self.find_member(full, prop)?;
                if doc.is_empty() {
                    format!("```csharp\n{decl_owner}.{prop} ({kind})\n```")
                } else {
                    format!("```csharp\n{decl_owner}.{prop} ({kind})\n```\n---\n{doc}")
                }
            } else {
                let (_, full) = self.tags.iter().find(|(n, _)| n == word)?;
                let t = self.types.get(full)?;
                if t.summary.is_empty() {
                    format!("```csharp\n{full}\n```")
                } else {
                    format!("```csharp\n{full}\n```\n---\n{}", t.summary)
                }
            }
        } else if let Some(&(_, desc)) = X_DIRECTIVES.iter().find(|&&(d, _)| d == word) {
            format!("```xml\n{word} (XAML directive)\n```\n---\n{desc}")
        } else if let Some((owner, prop)) = word.split_once('.') {
            let (_, full) = self.tags.iter().find(|(n, _)| n == owner)?;
            let t = self.types.get(full)?;
            if !t.attached.iter().any(|a| a == prop) {
                return None;
            }
            let doc = t
                .docs
                .get(&format!("A:{prop}"))
                .cloned()
                .unwrap_or_default();
            if doc.is_empty() {
                format!("```csharp\n{}.{prop} (attached property)\n```", t.name)
            } else {
                format!(
                    "```csharp\n{}.{prop} (attached property)\n```\n---\n{doc}",
                    t.name
                )
            }
        } else {
            let tag_name = before_word
                .split(char::is_whitespace)
                .next()?
                .trim_end_matches('/');
            let (_, full) = self.tags.iter().find(|(n, _)| n == tag_name)?;
            let (owner, kind, doc) = self.find_member(full, word)?;
            if doc.is_empty() {
                format!("```csharp\n{owner}.{word} ({kind})\n```")
            } else {
                format!("```csharp\n{owner}.{word} ({kind})\n```\n---\n{doc}")
            }
        };

        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: md,
            }),
            range: None,
        })
    }

    pub fn semantic_tokens(&self, text: &str) -> Vec<SemanticToken> {
        let mut line_starts = vec![0usize];
        let mut raw = Vec::new();

        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }

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
                    || X_DIRECTIVES.iter().any(|&(d, _)| d == attr)
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
    replace: Range,
) -> CompletionItem {
    CompletionItem {
        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
            range: replace,
            new_text: format!("{label}=\"$0\""),
        })),
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
