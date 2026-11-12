use crate::metadata::{
    enum_values_for_attr, TypeMeta, BINDING_SNIPPETS, EXTRA_TAGS, TAG_BASES, TAG_SNIPPETS,
    X_DIRECTIVES,
};
use crate::parser::{byte_range_to_lsp, inside_quotes, is_ident_char, scan, Ctx, Kind};
use std::collections::{HashMap, HashSet};
use tower_lsp::lsp_types::*;

pub struct EventAtCursor {
    pub tag_name: String,
    pub event_name: String,
    pub handler_name: String,
}

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
        let mut types: HashMap<String, TypeMeta> = serde_json::from_str(&data).ok()?;

        for &(name, full, summary) in EXTRA_TAGS {
            types.entry(full.to_string()).or_insert_with(|| TypeMeta {
                name: name.to_string(),
                namespace: "Avalonia".to_string(),
                base: None,
                properties: vec!["DataType".to_string()],
                attached: vec![],
                events: vec![],
                summary: summary.to_string(),
                docs: HashMap::new(),
            });
        }

        let mut seen_names = HashSet::new();
        let mut tags: Vec<(String, String)> = types
            .iter()
            .filter(|(full, t)| {
                !t.name.contains('`')
                    && (TAG_BASES
                        .iter()
                        .any(|base| Self::inherits(&types, full, base))
                        || EXTRA_TAGS.iter().any(|&(_, ef, _)| ef == full.as_str()))
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
                    label: label.clone(),
                    sort_text: Some(format!("1_{label}")),
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

                let mut items: Vec<CompletionItem> = Vec::new();

                // 1. Готовые UI-сниппеты
                if !has_close_bracket {
                    for &(label, desc, body) in TAG_SNIPPETS {
                        items.push(CompletionItem {
                            label: label.to_string(),
                            sort_text: Some(format!("0_{label}")),
                            kind: Some(CompletionItemKind::SNIPPET),
                            detail: Some(desc.to_string()),
                            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                                range: replace,
                                new_text: body.to_string(),
                            })),
                            insert_text_format: Some(InsertTextFormat::SNIPPET),
                            ..Default::default()
                        });
                    }
                }

                // 2. Стандартные контролы и теги
                for (name, full) in &self.tags {
                    let new_text = if has_close_bracket {
                        name.clone()
                    } else {
                        format!("{name}>$0</{name}>")
                    };
                    items.push(CompletionItem {
                        label: name.clone(),
                        sort_text: Some(format!("1_{name}")),
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
                    });
                }

                // 3. Вложенные свойства родителя (например, ListBox.ItemTemplate)
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
                ancestors,
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

                // 1. Контекстные Attached-свойства от открытых родителей (Grid.Column, DockPanel.Dock)
                for anc in ancestors.iter().rev() {
                    let anc_base = anc.split('.').next().unwrap_or(anc);
                    if let Some(t) = self.types.values().find(|t| t.name == anc_base) {
                        for a in &t.attached {
                            let label = format!("{}.{}", t.name, a);
                            if seen.insert(label.clone()) {
                                let mut item = attr_item(
                                    label.clone(),
                                    &format!("attached from parent <{}>", t.name),
                                    CompletionItemKind::PROPERTY,
                                    t.docs.get(&format!("A:{a}")),
                                    replace,
                                );
                                item.sort_text = Some(format!("0_{label}"));
                                items.push(item);
                            }
                        }
                    }
                }

                // 2. Собственные свойства и события контрола
                for t in self.chain(full) {
                    for p in &t.properties {
                        if p == "Item" || !seen.insert(p.clone()) {
                            continue;
                        }
                        let mut item = attr_item(
                            p.clone(),
                            &t.name,
                            CompletionItemKind::PROPERTY,
                            t.docs.get(&format!("P:{p}")),
                            replace,
                        );
                        item.sort_text = Some(format!("1_{p}"));
                        items.push(item);
                    }
                    for e in &t.events {
                        if !seen.insert(e.clone()) {
                            continue;
                        }
                        let mut item = attr_item(
                            e.clone(),
                            &t.name,
                            CompletionItemKind::EVENT,
                            t.docs.get(&format!("E:{e}")),
                            replace,
                        );
                        item.sort_text = Some(format!("2_{e}"));
                        items.push(item);
                    }
                }

                // 3. XAML-директивы (x:Name, x:DataType...)
                for &(dir, desc) in X_DIRECTIVES {
                    let doc_str = desc.to_string();
                    let mut item = attr_item(
                        dir.to_string(),
                        "XAML directive",
                        CompletionItemKind::KEYWORD,
                        Some(&doc_str),
                        replace,
                    );
                    item.sort_text = Some(format!("0_{dir}"));
                    items.push(item);
                }

                // 4. Остальные часто используемые Attached-свойства (или все при вводе точки)
                let has_dot = prefix.contains('.');
                for t in self.types.values() {
                    if !has_dot
                        && !matches!(
                            t.name.as_str(),
                            "Grid" | "DockPanel" | "Canvas" | "ToolTip" | "ScrollViewer"
                        )
                    {
                        continue;
                    }
                    for a in &t.attached {
                        let label = format!("{}.{}", t.name, a);
                        if seen.insert(label.clone()) {
                            let mut item = attr_item(
                                label.clone(),
                                "attached",
                                CompletionItemKind::PROPERTY,
                                t.docs.get(&format!("A:{a}")),
                                replace,
                            );
                            item.sort_text = Some(format!("3_{label}"));
                            items.push(item);
                        }
                    }
                }
                items
            }
            Ctx::AttrValue {
                tag: _,
                attr,
                prefix,
                replace,
            } => {
                let mut items = Vec::new();

                // 1. Если ещё не начали писать '{', предлагаем Enum-значения для конкретного атрибута
                if !prefix.starts_with('{') {
                    for &val in enum_values_for_attr(&attr) {
                        items.push(CompletionItem {
                            label: val.to_string(),
                            sort_text: Some(format!("0_{val}")),
                            kind: Some(CompletionItemKind::ENUM_MEMBER),
                            detail: Some(format!("{attr} value")),
                            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                                range: replace,
                                new_text: val.to_string(),
                            })),
                            ..Default::default()
                        });
                    }
                }

                // 2. Маркап-расширения ({Binding}, {StaticResource}...)
                for &(label, desc, snippet) in BINDING_SNIPPETS {
                    items.push(CompletionItem {
                        label: label.to_string(),
                        sort_text: Some(format!("1_{label}")),
                        kind: Some(CompletionItemKind::SNIPPET),
                        detail: Some(desc.to_string()),
                        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                            range: replace,
                            new_text: snippet.to_string(),
                        })),
                        insert_text_format: Some(InsertTextFormat::SNIPPET),
                        ..Default::default()
                    });
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

    pub fn diagnostics(&self, text: &str) -> Vec<Diagnostic> {
        let mut diags = Vec::new();
        let mut stack: Vec<(String, usize)> = Vec::new();

        for tag in scan(text) {
            match tag.kind {
                Kind::Open => stack.push((tag.name.clone(), tag.name_off)),
                Kind::SelfClose => {}
                Kind::Close => {
                    if let Some(pos) = stack.iter().rposition(|(n, _)| *n == tag.name) {
                        for (unclosed_name, unclosed_off) in stack.drain(pos + 1..) {
                            diags.push(Diagnostic {
                                range: byte_range_to_lsp(
                                    text,
                                    unclosed_off,
                                    unclosed_off + unclosed_name.len(),
                                ),
                                severity: Some(DiagnosticSeverity::ERROR),
                                source: Some("axaml-ls".to_string()),
                                message: format!("Незакрытый тег <{unclosed_name}>"),
                                ..Default::default()
                            });
                        }
                        stack.pop();
                    } else {
                        diags.push(Diagnostic {
                            range: byte_range_to_lsp(
                                text,
                                tag.name_off,
                                tag.name_off + tag.name.len(),
                            ),
                            severity: Some(DiagnosticSeverity::ERROR),
                            source: Some("axaml-ls".to_string()),
                            message: format!(
                                "Закрывающий тег </{}> не имеет соответствующего открывающего тега",
                                tag.name
                            ),
                            ..Default::default()
                        });
                    }
                    continue;
                }
            }

            // Пропускаем теги с кастомными XML-префиксами (например, local:MyControl)
            if tag.name.contains(':') {
                continue;
            }

            if let Some((owner, prop)) = tag.name.split_once('.') {
                if let Some((_, full)) = self.tags.iter().find(|(n, _)| n == owner) {
                    if self.find_member(full, prop).is_none()
                        && !self.is_attached(&tag.name)
                        && prop != "Resources"
                        && prop != "Styles"
                        && prop != "DataTemplates"
                    {
                        diags.push(Diagnostic {
                            range: byte_range_to_lsp(
                                text,
                                tag.name_off,
                                tag.name_off + tag.name.len(),
                            ),
                            severity: Some(DiagnosticSeverity::WARNING),
                            source: Some("axaml-ls".to_string()),
                            message: format!("Неизвестное свойство <{}> у {owner}", tag.name),
                            ..Default::default()
                        });
                    }
                }
                continue;
            }

            let Some((_, full)) = self.tags.iter().find(|(n, _)| *n == tag.name) else {
                diags.push(Diagnostic {
                    range: byte_range_to_lsp(text, tag.name_off, tag.name_off + tag.name.len()),
                    severity: Some(DiagnosticSeverity::WARNING),
                    source: Some("axaml-ls".to_string()),
                    message: format!("Неизвестный контрол или тег Avalonia: <{}>", tag.name),
                    ..Default::default()
                });
                continue;
            };

            let chain = self.chain(full);
            for attr in &tag.attrs {
                let aname = &attr.name;
                if aname == "xmlns"
                    || aname.starts_with("xmlns:")
                    || aname.starts_with("mc:")
                    || aname.starts_with("d:")
                    || X_DIRECTIVES.iter().any(|&(d, _)| d == aname)
                {
                    continue;
                }
                if aname.contains(':') {
                    continue;
                }

                let is_known = chain
                    .iter()
                    .any(|t| t.properties.contains(aname) || t.events.contains(aname))
                    || self.is_attached(aname)
                    || (tag.name == "DataTemplate" && aname == "DataType");

                if !is_known {
                    diags.push(Diagnostic {
                        range: byte_range_to_lsp(text, attr.name_off, attr.name_off + aname.len()),
                        severity: Some(DiagnosticSeverity::WARNING),
                        source: Some("axaml-ls".to_string()),
                        message: format!(
                            "Неизвестное свойство или событие '{aname}' у <{}>",
                            tag.name
                        ),
                        ..Default::default()
                    });
                }
            }
        }

        for (unclosed_name, unclosed_off) in stack {
            diags.push(Diagnostic {
                range: byte_range_to_lsp(text, unclosed_off, unclosed_off + unclosed_name.len()),
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some("axaml-ls".to_string()),
                message: format!("Незакрытый тег <{unclosed_name}>"),
                ..Default::default()
            });
        }

        diags
    }

    pub fn event_at_offset(&self, text: &str, off: usize) -> Option<EventAtCursor> {
        for tag in scan(text) {
            if tag.kind == Kind::Close {
                continue;
            }
            let Some((_, full)) = self.tags.iter().find(|(n, _)| *n == tag.name) else {
                continue;
            };
            let chain = self.chain(full);

            for attr in &tag.attrs {
                let Some(val) = &attr.value else { continue };
                let Some(val_off) = attr.value_off else {
                    continue;
                };
                let attr_end = val_off + val.len() + 1;
                if off >= attr.name_off && off <= attr_end {
                    let is_event = chain.iter().any(|t| t.events.contains(&attr.name));
                    if is_event && !val.is_empty() && !val.starts_with('{') {
                        return Some(EventAtCursor {
                            tag_name: tag.name.clone(),
                            event_name: attr.name.clone(),
                            handler_name: val.clone(),
                        });
                    }
                }
            }
        }
        None
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

            for attr in &tag.attrs {
                let aname = &attr.name;
                let off = attr.name_off;
                let ty = if chain.iter().any(|t| t.events.contains(aname)) {
                    1
                } else if chain.iter().any(|t| t.properties.contains(aname))
                    || self.is_attached(aname)
                    || X_DIRECTIVES.iter().any(|&(d, _)| d == aname)
                {
                    0
                } else {
                    continue;
                };

                let line = line_starts.partition_point(|&s| s <= off) - 1;
                let col = text[line_starts[line]..off].encode_utf16().count() as u32;
                let len = aname.encode_utf16().count() as u32;
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
