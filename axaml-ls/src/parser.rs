use tower_lsp::lsp_types::{Position, Range};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Open,
    Close,
    SelfClose,
}

pub struct AttrSpan {
    pub name: String,
    pub name_off: usize,
    pub value: Option<String>,
    pub value_off: Option<usize>,
}

pub struct Tag {
    pub kind: Kind,
    pub name: String,
    pub name_off: usize,
    pub attrs: Vec<AttrSpan>,
}

#[allow(clippy::enum_variant_names)]
pub enum Ctx {
    None,
    Tag {
        prefix: String,
        parent: Option<String>,
        replace: Range,
        has_close_bracket: bool,
    },
    Close {
        name: String,
        replace: Range,
        has_close_bracket: bool,
    },
    Attr {
        tag: String,
        prefix: String,
        ancestors: Vec<String>,
        replace: Range,
    },
    AttrValue {
        tag: String,
        attr: String,
        prefix: String,
        replace: Range,
    },
}

pub fn scan(text: &str) -> Vec<Tag> {
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
            break;
        }

        let inner_start = i + 1;
        let inner = &text[inner_start..j];
        i = j + 1;

        if inner.starts_with('?') || inner.starts_with('!') {
            continue;
        }
        if let Some(rest) = inner.strip_prefix('/') {
            let trimmed = rest.trim();
            let ws_lead = rest.len() - rest.trim_start().len();
            out.push(Tag {
                kind: Kind::Close,
                name: trimmed.to_string(),
                name_off: inner_start + 1 + ws_lead,
                attrs: vec![],
            });
            continue;
        }

        let name_end = inner
            .find(|c: char| c.is_whitespace() || c == '/')
            .unwrap_or(inner.len());
        let name = inner[..name_end].to_string();

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
            let attr_name = if k > s {
                Some((inner[s..k].to_string(), inner_start + s))
            } else {
                None
            };

            while k < ib.len() && ib[k].is_ascii_whitespace() {
                k += 1;
            }
            let mut val_info = None;
            if k < ib.len() && ib[k] == b'=' {
                k += 1;
                while k < ib.len() && ib[k].is_ascii_whitespace() {
                    k += 1;
                }
                if k < ib.len() && (ib[k] == b'"' || ib[k] == b'\'') {
                    let q = ib[k];
                    k += 1;
                    let val_start = k;
                    while k < ib.len() && ib[k] != q {
                        k += 1;
                    }
                    val_info = Some((inner[val_start..k].to_string(), inner_start + val_start));
                    if k < ib.len() {
                        k += 1;
                    }
                } else {
                    let val_start = k;
                    while k < ib.len() && !ib[k].is_ascii_whitespace() {
                        k += 1;
                    }
                    val_info = Some((inner[val_start..k].to_string(), inner_start + val_start));
                }
            } else if k == s {
                k += 1;
            }

            if let Some((aname, aoff)) = attr_name {
                attrs.push(AttrSpan {
                    name: aname,
                    name_off: aoff,
                    value: val_info.as_ref().map(|(v, _)| v.clone()),
                    value_off: val_info.map(|(_, o)| o),
                });
            }
        }

        let self_close = inner.trim_end().ends_with('/');
        out.push(Tag {
            kind: if self_close {
                Kind::SelfClose
            } else {
                Kind::Open
            },
            name,
            name_off: inner_start,
            attrs,
        });
    }
    out
}

pub fn open_stack(text: &str) -> Vec<String> {
    let mut stack = Vec::new();
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

pub fn offset(text: &str, pos: Position) -> Option<usize> {
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

pub fn offset_to_position(text: &str, off: usize) -> Position {
    let clamped = off.min(text.len());
    let before = &text[..clamped];
    let line = before.bytes().filter(|&b| b == b'\n').count() as u32;
    let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let character = before[line_start..].encode_utf16().count() as u32;
    Position { line, character }
}

pub fn byte_range_to_lsp(text: &str, start: usize, end: usize) -> Range {
    Range {
        start: offset_to_position(text, start),
        end: offset_to_position(text, end),
    }
}

pub fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.' || c == ':' || c == '-'
}

pub fn inside_quotes(s: &str) -> bool {
    quote_context(s).is_some()
}

/// Если мы находимся внутри открытой кавычки атрибута, возвращает (attr_name, value_prefix)
pub fn quote_context(s: &str) -> Option<(String, String)> {
    let mut quote: Option<(char, usize)> = None;
    for (i, c) in s.char_indices() {
        match quote {
            Some((q, _)) if c == q => quote = None,
            None if c == '"' || c == '\'' => quote = Some((c, i)),
            _ => {}
        }
    }
    let (_, q_idx) = quote?;
    let before_quote = s[..q_idx].trim_end();
    let before_eq = before_quote.strip_suffix('=')?.trim_end();
    let attr_name: String = before_eq
        .chars()
        .rev()
        .take_while(|&c| is_ident_char(c))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let value_prefix = s[q_idx + 1..].to_string();
    Some((attr_name, value_prefix))
}

pub fn format_document(text: &str, tab_size: usize, insert_spaces: bool) -> String {
    let indent_unit = if insert_spaces {
        " ".repeat(tab_size.max(1))
    } else {
        "\t".to_string()
    };

    #[derive(Debug)]
    enum Token {
        Comment(String),
        Open {
            name: String,
            attrs: Vec<(String, Option<String>)>,
            self_close: bool,
        },
        Close(String),
        Text(String),
    }

    let b = text.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < b.len() {
        if b[i] == b'<' {
            if text[i..].starts_with("<!--") {
                if let Some(end) = text[i..].find("-->") {
                    tokens.push(Token::Comment(text[i..i + end + 3].trim().to_string()));
                    i += end + 3;
                    continue;
                }
            }
            let mut j = i + 1;
            let mut quote: Option<u8> = None;
            while j < b.len() {
                match quote {
                    Some(q) if b[j] == q => quote = None,
                    None if b[j] == b'"' || b[j] == b'\'' => quote = Some(b[j]),
                    None if b[j] == b'>' => break,
                    _ => {}
                }
                j += 1;
            }
            if j >= b.len() {
                let rem = text[i..].trim();
                if !rem.is_empty() {
                    tokens.push(Token::Text(rem.to_string()));
                }
                break;
            }

            let inner = text[i + 1..j].trim();
            i = j + 1;

            if inner.starts_with('?') || inner.starts_with('!') {
                tokens.push(Token::Comment(format!("<{inner}>")));
                continue;
            }

            if let Some(rest) = inner.strip_prefix('/') {
                tokens.push(Token::Close(rest.trim().to_string()));
                continue;
            }

            let self_close = inner.ends_with('/');
            let inner_clean = if self_close {
                inner[..inner.len() - 1].trim_end()
            } else {
                inner
            };

            let name_end = inner_clean
                .find(|c: char| c.is_whitespace())
                .unwrap_or(inner_clean.len());
            let name = inner_clean[..name_end].to_string();

            let mut attrs = Vec::new();
            let ib = inner_clean.as_bytes();
            let mut k = name_end;
            while k < ib.len() {
                while k < ib.len() && ib[k].is_ascii_whitespace() {
                    k += 1;
                }
                if k >= ib.len() {
                    break;
                }
                let s = k;
                while k < ib.len() && !ib[k].is_ascii_whitespace() && ib[k] != b'=' {
                    k += 1;
                }
                let aname = inner_clean[s..k].to_string();
                while k < ib.len() && ib[k].is_ascii_whitespace() {
                    k += 1;
                }
                let mut aval = None;
                if k < ib.len() && ib[k] == b'=' {
                    k += 1;
                    while k < ib.len() && ib[k].is_ascii_whitespace() {
                        k += 1;
                    }
                    if k < ib.len() && (ib[k] == b'"' || ib[k] == b'\'') {
                        let q = ib[k];
                        k += 1;
                        let vs = k;
                        while k < ib.len() && ib[k] != q {
                            k += 1;
                        }
                        aval = Some(inner_clean[vs..k].to_string());
                        if k < ib.len() {
                            k += 1;
                        }
                    } else {
                        let vs = k;
                        while k < ib.len() && !ib[k].is_ascii_whitespace() {
                            k += 1;
                        }
                        aval = Some(inner_clean[vs..k].to_string());
                    }
                }
                if !aname.is_empty() {
                    attrs.push((aname, aval));
                }
            }

            tokens.push(Token::Open {
                name,
                attrs,
                self_close,
            });
        } else {
            let next_lt = text[i..].find('<').map(|o| i + o).unwrap_or(b.len());
            let t = text[i..next_lt].trim();
            if !t.is_empty() {
                tokens.push(Token::Text(t.to_string()));
            }
            i = next_lt;
        }
    }

    let mut out = String::new();
    let mut depth = 0usize;
    let mut idx = 0;

    let format_attrs = |attrs: &[(String, Option<String>)],
                        tag_indent: &str,
                        attr_indent: &str,
                        is_root: bool|
     -> String {
        if attrs.is_empty() {
            return String::new();
        }
        let has_xmlns = attrs
            .iter()
            .any(|(k, _)| k == "xmlns" || k.starts_with("xmlns:"));
        let multiline = (is_root && attrs.len() > 1) || has_xmlns || attrs.len() > 4;

        if !multiline {
            let joined: Vec<String> = attrs
                .iter()
                .map(|(k, v)| match v {
                    Some(val) => format!("{k}=\"{val}\""),
                    None => k.clone(),
                })
                .collect();
            format!(" {}", joined.join(" "))
        } else {
            let mut s = String::new();
            for (k, v) in attrs {
                s.push('\n');
                s.push_str(tag_indent);
                s.push_str(attr_indent);
                match v {
                    Some(val) => s.push_str(&format!("{k}=\"{val}\"")),
                    None => s.push_str(k),
                }
            }
            s
        }
    };

    while idx < tokens.len() {
        let indent = indent_unit.repeat(depth);
        match &tokens[idx] {
            Token::Comment(c) => {
                out.push_str(&indent);
                out.push_str(c);
                out.push('\n');
                idx += 1;
            }
            Token::Text(t) => {
                out.push_str(&indent);
                out.push_str(t);
                out.push('\n');
                idx += 1;
            }
            Token::Open {
                name,
                attrs,
                self_close,
            } => {
                let is_root = depth == 0;
                let attr_str = format_attrs(attrs, &indent, &indent_unit, is_root);

                // 1. Если за открывающим сразу идёт его закрывающий (<Tag></Tag>) -> схлопываем в <Tag />
                if !*self_close {
                    if let Some(Token::Close(cname)) = tokens.get(idx + 1) {
                        if cname == name {
                            out.push_str(&indent);
                            out.push('<');
                            out.push_str(name);
                            out.push_str(&attr_str);
                            out.push_str(" />\n");
                            idx += 2;
                            continue;
                        }
                    }
                    // 2. Если внутри только короткий однострочный текст (<TextBlock>Текст</TextBlock>)
                    if let (Some(Token::Text(txt)), Some(Token::Close(cname))) =
                        (tokens.get(idx + 1), tokens.get(idx + 2))
                    {
                        if cname == name && !txt.contains('\n') && txt.len() <= 80 {
                            out.push_str(&indent);
                            out.push('<');
                            out.push_str(name);
                            out.push_str(&attr_str);
                            out.push('>');
                            out.push_str(txt);
                            out.push_str("</");
                            out.push_str(name);
                            out.push_str(">\n");
                            idx += 3;
                            continue;
                        }
                    }
                }

                out.push_str(&indent);
                out.push('<');
                out.push_str(name);
                out.push_str(&attr_str);
                if *self_close {
                    out.push_str(" />\n");
                } else {
                    out.push_str(">\n");
                    depth += 1;
                }
                idx += 1;
            }
            Token::Close(name) => {
                depth = depth.saturating_sub(1);
                let close_indent = indent_unit.repeat(depth);
                out.push_str(&close_indent);
                out.push_str("</");
                out.push_str(name);
                out.push_str(">\n");
                idx += 1;
            }
        }
    }

    out
}

pub fn context(text: &str, off: usize, pos: Position) -> Ctx {
    let before = &text[..off];
    let after = &text[off..];

    let Some(lt) = before.rfind('<') else {
        return Ctx::None;
    };
    if before[lt..].contains('>') {
        return Ctx::None;
    }
    let seg = &before[lt + 1..];

    let after_ident_bytes: usize = after
        .chars()
        .take_while(|&c| is_ident_char(c))
        .map(char::len_utf8)
        .sum();
    let after_ident_utf16: u32 = after[..after_ident_bytes]
        .chars()
        .map(|c| c.len_utf16() as u32)
        .sum();
    let rest_after_ident = after[after_ident_bytes..].trim_start();
    let has_close_bracket = rest_after_ident.starts_with('>') || rest_after_ident.starts_with("/>");

    let end_pos = Position {
        line: pos.line,
        character: pos.character + after_ident_utf16,
    };

    if let Some(rest) = seg.strip_prefix('/') {
        if rest.contains(char::is_whitespace) {
            return Ctx::None;
        }
        let prefix_utf16 = rest.encode_utf16().count() as u32;
        let replace = Range {
            start: Position {
                line: pos.line,
                character: pos.character.saturating_sub(prefix_utf16),
            },
            end: end_pos,
        };
        return match open_stack(&before[..lt]).pop() {
            Some(name) => Ctx::Close {
                name,
                replace,
                has_close_bracket,
            },
            None => Ctx::None,
        };
    }
    if seg.starts_with(['!', '?']) {
        return Ctx::None;
    }

    match seg.find(char::is_whitespace) {
        None => {
            if seg.contains('/') {
                return Ctx::None;
            }
            let prefix_utf16 = seg.encode_utf16().count() as u32;
            let replace = Range {
                start: Position {
                    line: pos.line,
                    character: pos.character.saturating_sub(prefix_utf16),
                },
                end: end_pos,
            };
            let parent = open_stack(&before[..lt]).pop();
            Ctx::Tag {
                prefix: seg.to_string(),
                parent,
                replace,
                has_close_bracket,
            }
        }
        Some(ws) => {
            let tag_name = seg[..ws].trim_end_matches('/').to_string();
            if let Some((attr, val_prefix)) = quote_context(seg) {
                let prefix_utf16 = val_prefix.encode_utf16().count() as u32;
                let replace = Range {
                    start: Position {
                        line: pos.line,
                        character: pos.character.saturating_sub(prefix_utf16),
                    },
                    end: pos,
                };
                return Ctx::AttrValue {
                    tag: tag_name,
                    attr,
                    prefix: val_prefix,
                    replace,
                };
            }
            let prefix: String = seg
                .chars()
                .rev()
                .take_while(|&c| is_ident_char(c))
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let prefix_utf16 = prefix.encode_utf16().count() as u32;
            let replace = Range {
                start: Position {
                    line: pos.line,
                    character: pos.character.saturating_sub(prefix_utf16),
                },
                end: end_pos,
            };
            let ancestors = open_stack(&before[..lt]);
            Ctx::Attr {
                tag: tag_name,
                prefix,
                ancestors,
                replace,
            }
        }
    }
}
