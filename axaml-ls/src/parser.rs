use tower_lsp::lsp_types::{Position, Range};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Open,
    Close,
    SelfClose,
}

pub struct Tag {
    pub kind: Kind,
    pub name: String,
    pub attrs: Vec<(String, usize)>,
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
            out.push(Tag {
                kind: Kind::Close,
                name: rest.trim().to_string(),
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
                k += 1;
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

pub fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.' || c == ':'
}

pub fn inside_quotes(s: &str) -> bool {
    let mut quote = None;
    for c in s.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            None if c == '"' || c == '\'' => quote = Some(c),
            _ => {}
        }
    }
    quote.is_some()
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
            if inside_quotes(seg) {
                return Ctx::None;
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
            let tag_name = seg[..ws].trim_end_matches('/').to_string();
            Ctx::Attr {
                tag: tag_name,
                prefix,
                replace,
            }
        }
    }
}
