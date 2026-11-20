use crate::parser::{byte_range_to_lsp, scan, Kind};
use crate::index::Index;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use tower_lsp::lsp_types::*;

pub const CREATE_HANDLER: &str = "axaml.createHandler";
pub const CREATE_VIEW: &str = "axaml.createView";
pub const INIT_VIEW_CS: &str = "axaml.initViewCs";

struct HandlerCall {
    tag: String,
    event: String,
    handler: String,
    xname: Option<String>,
    value_start: usize,
    value_end: usize,
}

fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some(f) if f.is_alphabetic() || f == '_')
        && c.all(|x| x.is_alphanumeric() || x == '_')
}

fn position_of(text: &str, off: usize) -> Position {
    let line = text[..off].matches('\n').count() as u32;
    let ls = text[..off].rfind('\n').map_or(0, |n| n + 1);
    let col = text[ls..off].encode_utf16().count() as u32;
    Position::new(line, col)
}

/// курсор внутри значения атрибута: Event="Handler"
fn handler_at(text: &str, off: usize) -> Option<HandlerCall> {
    let lt = text.get(..off)?.rfind('<')?;
    let rest = &text[lt..];
    if rest.starts_with("</") || rest.starts_with("<!") || rest.starts_with("<?") {
        return None;
    }

    let b = text.as_bytes();
    let mut j = lt + 1;
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
    if off > j {
        return None;
    }

    let inner_start = lt + 1;
    let inner = &text[inner_start..j];
    let name_end = inner.find(|c: char| c.is_whitespace())?;
    let tag = inner[..name_end].to_string();
    let xname = inner.find("x:Name=\"").and_then(|i| {
        let s = i + 8;
        let e = inner[s..].find('"')?;
        Some(inner[s..s + e].to_string())
    });

    let ib = inner.as_bytes();
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
        let name = &inner[s..k];
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
                let vs = k + 1;
                let mut ve = vs;
                while ve < ib.len() && ib[ve] != q {
                    ve += 1;
                }
                let (abs_s, abs_e) = (inner_start + vs, inner_start + ve);
                if !name.is_empty() && off >= abs_s && off <= abs_e {
                    return Some(HandlerCall {
                        tag,
                        event: name.to_string(),
                        handler: inner[vs..ve].trim().to_string(),
                        xname,
                        value_start: abs_s,
                        value_end: abs_e,
                    });
                }
                k = ve + 1;
            } else {
                while k < ib.len() && !ib[k].is_ascii_whitespace() {
                    k += 1;
                }
            }
        } else if k == s {
            k += 1;
        }
    }
    None
}

/// есть ли в C#-тексте метод с таким именем
fn has_method(text: &str, name: &str) -> bool {
    let mut from = 0;
    while let Some(i) = text[from..].find(name) {
        let s = from + i;
        let e = s + name.len();
        let prev_ok = text[..s]
            .chars()
            .next_back()
            .map_or(true, |c| !(c.is_alphanumeric() || c == '_'));
        let next = text[e..].trim_start().chars().next();
        if prev_ok && next == Some('(') {
            return true;
        }
        from = e;
    }
    false
}

/// байтовый сдвиг закрывающей скобки класса
fn class_end(text: &str, class: &str) -> Option<usize> {
    let pat = format!("class {class}");
    let mut from = 0;
    let start = loop {
        let i = from + text[from..].find(&pat)?;
        let after = i + pat.len();
        let ends = text[after..]
            .chars()
            .next()
            .map_or(true, |c| !(c.is_alphanumeric() || c == '_'));
        if ends {
            break after;
        }
        from = after;
    };

    let b = text.as_bytes();
    let open = start + text[start..].find('{')?;
    let mut depth = 0i32;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'/' if i + 1 < b.len() && b[i + 1] == b'/' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < b.len() && b[i + 1] == b'*' => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// байтовый сдвиг, куда вставить новый `using` (после последнего существующего)
fn using_insert_offset(text: &str) -> usize {
    let mut off = 0;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        if t.starts_with("using ") {
            at = off + line.len();
        } else if t.starts_with("namespace ") || t.contains("class ") {
            break;
        }
        off += line.len();
    }
    at
}

struct CsTarget {
    path: PathBuf,
    uri: Url,
    text: String,
    class: String,
}

/// MainWindow.axaml -> MainWindow.axaml.cs, класс берём из x:Class
fn cs_target(docs: &HashMap<Url, String>, uri: &Url, xaml: &str) -> Option<CsTarget> {
    let axaml = uri.to_file_path().ok()?;
    let ext = axaml.extension()?.to_str()?;
    if ext != "axaml" && ext != "xaml" {
        return None;
    }
    let path = PathBuf::from(format!("{}.cs", axaml.display()));
    let cs_uri = Url::from_file_path(&path).ok()?;
    let text = match docs.get(&cs_uri) {
        Some(t) => t.clone(),
        None => std::fs::read_to_string(&path).ok()?,
    };
    let class = xaml.find("x:Class=\"").and_then(|i| {
        let s = i + 9;
        let e = xaml[s..].find('"')?;
        xaml[s..s + e].rsplit('.').next().map(str::to_string)
    })?;
    Some(CsTarget {
        path,
        uri: cs_uri,
        text,
        class,
    })
}

/// (пространство имён, имя типа) аргументов события из индекса
fn event_arg(index: &Index, tag: &str, event: &str) -> Option<(String, String)> {
    let full = index.tags.iter().find(|(n, _)| n == tag).map(|(_, f)| f)?;
    let delegate = index
        .chain(full)
        .into_iter()
        .find(|t| t.events.iter().any(|e| e == event))
        .and_then(|t| t.event_args.get(event))?;

    let arg_full = match delegate.strip_suffix('>').and_then(|d| d.split_once('<')) {
        Some((base, inner)) if base.starts_with("System.EventHandler") && !inner.contains('<') => {
            inner.to_string()
        }
        _ => "System.EventArgs".to_string(),
    };
    Some(match arg_full.rsplit_once('.') {
        Some((n, s)) => (n.to_string(), s.to_string()),
        None => (String::new(), arg_full),
    })
}

/// что и куда вставить в .axaml.cs: список (байтовый сдвиг, текст)
fn insertions(
    t: &CsTarget,
    handler: &str,
    arg_ns: &str,
    arg_name: &str,
) -> Option<Vec<(usize, String)>> {
    if has_method(&t.text, handler) {
        return None;
    }
    let close = class_end(&t.text, &t.class)?;

    let line_start = t.text[..close].rfind('\n').map_or(0, |n| n + 1);
    let before = &t.text[line_start..close];
    let (at, indent) = if before.trim().is_empty() {
        (line_start, before.to_string())
    } else {
        (close, String::new())
    };
    let bi = format!("{indent}    ");
    let method =
        format!("\n{bi}private void {handler}(object? sender, {arg_name} e)\n{bi}{{\n{bi}}}\n");

    let mut out = vec![(at, method)];
    if !arg_ns.is_empty() && arg_ns != "System" && !t.text.contains(&format!("using {arg_ns};")) {
        out.push((using_insert_offset(&t.text), format!("using {arg_ns};\n")));
    }
    Some(out)
}

fn to_edits(text: &str, ins: &[(usize, String)]) -> Vec<TextEdit> {
    ins.iter()
        .map(|(o, s)| {
            let p = position_of(text, *o);
            TextEdit {
                range: Range::new(p, p),
                new_text: s.clone(),
            }
        })
        .collect()
}

fn apply_insertions(text: &str, ins: &[(usize, String)]) -> String {
    let mut v: Vec<&(usize, String)> = ins.iter().collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    let mut s = text.to_string();
    for (o, t) in v {
        s.insert_str(*o, t);
    }
    s
}

/// имя в стиле VS: Button_Click или saveBtn_Click, с числом если занято
fn suggest_name(call: &HandlerCall, cs_text: &str) -> String {
    let base = match &call.xname {
        Some(n) if is_ident(n) => n.clone(),
        _ => call
            .tag
            .rsplit(':')
            .next()
            .unwrap_or(&call.tag)
            .replace('.', "_"),
    };
    let mut name = format!("{base}_{}", call.event);
    let mut n = 2;
    while has_method(cs_text, &name) {
        name = format!("{base}_{}{n}", call.event);
        n += 1;
    }
    name
}

/// код-экшен: создать метод-обработчик
pub fn create_handler(
    index: &Index,
    docs: &HashMap<Url, String>,
    uri: &Url,
    text: &str,
    off: usize,
) -> Option<CodeAction> {
    let call = handler_at(text, off)?;
    let (arg_ns, arg_name) = event_arg(index, &call.tag, &call.event)?;
    let t = cs_target(docs, uri, text)?;

    let generated = call.handler.is_empty();
    let handler = if generated {
        suggest_name(&call, &t.text)
    } else {
        call.handler.clone()
    };
    if !is_ident(&handler) {
        return None;
    }
    let ins = insertions(&t, &handler, &arg_ns, &arg_name)?;

    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    changes.insert(t.uri.clone(), to_edits(&t.text, &ins));
    if generated {
        changes.insert(
            uri.clone(),
            vec![TextEdit {
                range: Range::new(
                    position_of(text, call.value_start),
                    position_of(text, call.value_end),
                ),
                new_text: handler.clone(),
            }],
        );
    }

    let file = t
        .path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("code-behind")
        .to_string();
    Some(CodeAction {
        title: format!("Создать обработчик {handler} в {file}"),
        kind: Some(CodeActionKind::QUICKFIX),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }),
        ..Default::default()
    })
}

/// пункт подсказки в пустом значении события: имя + команда создания метода
pub fn handler_completion(
    index: &Index,
    docs: &HashMap<Url, String>,
    uri: &Url,
    text: &str,
    off: usize,
) -> Option<CompletionItem> {
    let call = handler_at(text, off)?;
    if !call.handler.is_empty() {
        return None;
    }
    event_arg(index, &call.tag, &call.event)?;
    let t = cs_target(docs, uri, text)?;
    let name = suggest_name(&call, &t.text);
    let file = t
        .path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("code-behind")
        .to_string();

    Some(CompletionItem {
        label: name.clone(),
        kind: Some(CompletionItemKind::METHOD),
        detail: Some(format!("создать обработчик в {file}")),
        sort_text: Some("0".to_string()),
        insert_text: Some(name.clone()),
        command: Some(Command {
            title: "Создать обработчик".to_string(),
            command: CREATE_HANDLER.to_string(),
            arguments: Some(vec![
                json!(uri.as_str()),
                json!(call.tag),
                json!(call.event),
                json!(name),
            ]),
        }),
        ..Default::default()
    })
}

/// выполнение команды: правка для клиента, путь файла и его новое содержимое (запасной путь)
pub fn run_create_handler(
    index: &Index,
    docs: &HashMap<Url, String>,
    uri: &Url,
    tag: &str,
    event: &str,
    handler: &str,
) -> Option<(WorkspaceEdit, PathBuf, String)> {
    if !is_ident(handler) {
        return None;
    }
    let xaml = docs.get(uri)?;
    let (arg_ns, arg_name) = event_arg(index, tag, event)?;
    let t = cs_target(docs, uri, xaml)?;
    let ins = insertions(&t, handler, &arg_ns, &arg_name)?;
    let new_text = apply_insertions(&t.text, &ins);

    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    changes.insert(t.uri.clone(), to_edits(&t.text, &ins));
    Some((
        WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        },
        t.path,
        new_text,
    ))
}

/// диагностика: предупреждения на события, чьих обработчиков нет в .axaml.cs
pub fn missing_handler_diagnostics(
    index: &Index,
    docs: &HashMap<Url, String>,
    uri: &Url,
    text: &str,
) -> Vec<Diagnostic> {
    let Some(t) = cs_target(docs, uri, text) else {
        return vec![];
    };
    let mut diags = Vec::new();
    for tag in scan(text) {
        if tag.kind == Kind::Close {
            continue;
        }
        let Some((_, full)) = index.tags.iter().find(|(n, _)| *n == tag.name) else {
            continue;
        };
        let chain = index.chain(full);
        for attr in &tag.attrs {
            let Some(val) = &attr.value else { continue };
            let Some(val_off) = attr.value_off else { continue };
            let trimmed = val.trim();
            if trimmed.is_empty() || trimmed.starts_with('{') {
                continue;
            }
            if chain.iter().any(|tm| tm.events.contains(&attr.name)) {
                if !is_ident(trimmed) {
                    diags.push(Diagnostic {
                        range: byte_range_to_lsp(text, val_off, val_off + val.len()),
                        severity: Some(DiagnosticSeverity::ERROR),
                        source: Some("axaml-ls".to_string()),
                        message: format!("Некорректное имя обработчика события: '{trimmed}'"),
                        ..Default::default()
                    });
                } else if !has_method(&t.text, trimmed) {
                    diags.push(Diagnostic {
                        range: byte_range_to_lsp(text, val_off, val_off + val.len()),
                        severity: Some(DiagnosticSeverity::WARNING),
                        source: Some("axaml-ls".to_string()),
                        message: format!(
                            "Обработчик '{trimmed}' не найден в {}",
                            t.path.file_name().and_then(|f| f.to_str()).unwrap_or("code-behind")
                        ),
                        ..Default::default()
                    });
                }
            }
        }
    }
    diags
}

/// переход к классу в .axaml.cs при клике на x:Class="..."
pub fn goto_x_class(docs: &HashMap<Url, String>, uri: &Url, text: &str, off: usize) -> Option<Location> {
    let call = handler_at(text, off)?;
    if call.event != "x:Class" {
        return None;
    }
    let t = cs_target(docs, uri, text)?;
    let pat = format!("class {}", t.class);
    let idx = t.text.find(&pat)?;
    let start = idx + 6;
    let end = start + t.class.len();
    Some(Location {
        uri: t.uri,
        range: byte_range_to_lsp(&t.text, start, end),
    })
}

/// Определяет C# namespace по .csproj и пути к папке
fn infer_namespace(dir: &std::path::Path) -> String {
    let mut cur = Some(dir);
    while let Some(d) = cur {
        if let Ok(entries) = std::fs::read_dir(d) {
            if let Some(csproj) = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .find(|p| p.extension().is_some_and(|ext| ext == "csproj"))
            {
                let root_ns = csproj
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("App")
                    .to_string();
                if let Ok(rel) = dir.strip_prefix(d) {
                    let sub: Vec<String> = rel
                        .components()
                        .filter_map(|c| c.as_os_str().to_str().map(str::to_string))
                        .collect();
                    if sub.is_empty() {
                        return root_ns;
                    }
                    return format!("{root_ns}.{}", sub.join("."));
                }
                return root_ns;
            }
        }
        cur = d.parent();
    }
    "App".to_string()
}

/// Подбирает свободное имя файла в папке: NewUserControl, NewUserControl1...
fn next_free_view_name(dir: &std::path::Path, base: &str) -> String {
    let mut name = base.to_string();
    let mut idx = 1;
    while dir.join(format!("{name}.axaml")).exists() {
        name = format!("{base}{idx}");
        idx += 1;
    }
    name
}

/// Code Actions для создания нового UserControl или Window в текущей папке
pub fn create_view_actions(uri: &Url, text: &str, off: usize) -> Vec<CodeAction> {
    let Ok(axaml_path) = uri.to_file_path() else {
        return vec![];
    };
    let Some(dir) = axaml_path.parent() else {
        return vec![];
    };

    // Если курсор стоит на теге с префиксом (например <local:MyCard />), предлагаем имя прямо из него
    let tag_hint = scan(text)
        .into_iter()
        .find(|t| t.kind != Kind::Close && off >= t.name_off && off <= t.name_off + t.name.len())
        .and_then(|t| t.name.split_once(':').map(|(_, name)| name.to_string()))
        .filter(|name| is_ident(name) && !dir.join(format!("{name}.axaml")).exists());

    let uc_name = tag_hint
        .clone()
        .unwrap_or_else(|| next_free_view_name(dir, "NewUserControl"));
    let win_name = tag_hint
        .unwrap_or_else(|| next_free_view_name(dir, "NewWindow"));

    let mut out = Vec::new();
    for (kind, name, title) in [
        ("UserControl", uc_name, "Создать Avalonia UserControl"),
        ("Window", win_name, "Создать Avalonia Window"),
    ] {
        out.push(CodeAction {
            title: format!("{title}: {name} (.axaml + .axaml.cs)"),
            kind: Some(CodeActionKind::REFACTOR),
            command: Some(Command {
                title: title.to_string(),
                command: CREATE_VIEW.to_string(),
                arguments: Some(vec![json!(uri.as_str()), json!(kind), json!(name)]),
            }),
            ..Default::default()
        });
    }
    out
}

/// Создаёт пару файлов <Name>.axaml и <Name>.axaml.cs
pub fn run_create_view(uri: &Url, kind: &str, name: &str) -> Option<PathBuf> {
    if !is_ident(name) || !matches!(kind, "UserControl" | "Window") {
        return None;
    }
    let axaml_path = uri.to_file_path().ok()?;
    let dir = axaml_path.parent()?;
    let ns = infer_namespace(dir);

    let new_axaml = dir.join(format!("{name}.axaml"));
    let new_cs = dir.join(format!("{name}.axaml.cs"));
    if new_axaml.exists() || new_cs.exists() {
        return None;
    }

    let size_attrs = if kind == "Window" {
        format!("\n        Width=\"800\" Height=\"450\"\n        Title=\"{name}\"")
    } else {
        "\n             d:DesignWidth=\"400\" d:DesignHeight=\"250\"".to_string()
    };

    let axaml_content = format!(
        "<{kind} xmlns=\"https://github.com/avaloniaui\"\n\
         \x20       xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n\
         \x20       xmlns:d=\"http://schemas.microsoft.com/expression/blend/2008\"\n\
         \x20       xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"\n\
         \x20       mc:Ignorable=\"d\"{size_attrs}\n\
         \x20       x:Class=\"{ns}.{name}\">\n\
         \x20   <Grid>\n\
         \x20       \n\
         \x20   </Grid>\n\
         </{kind}>\n"
    );

    let cs_content = format!(
        "using Avalonia.Controls;\n\n\
         namespace {ns};\n\n\
         public partial class {name} : {kind}\n\
         {{\n\
         \x20   public {name}()\n\
         \x20   {{\n\
         \x20       InitializeComponent();\n\
         \x20   }}\n\
         }}\n"
    );

    std::fs::write(&new_axaml, axaml_content).ok()?;
    std::fs::write(&new_cs, cs_content).ok()?;
    Some(new_axaml)
}

fn build_view_texts(ns: &str, kind: &str, name: &str) -> (String, String) {
    let size_attrs = if kind == "Window" {
        format!("\n        Width=\"800\" Height=\"450\"\n        Title=\"{name}\"")
    } else {
        "\n             d:DesignWidth=\"400\" d:DesignHeight=\"250\"".to_string()
    };

    let axaml_content = format!(
        "<{kind} xmlns=\"https://github.com/avaloniaui\"\n\
         \x20       xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n\
         \x20       xmlns:d=\"http://schemas.microsoft.com/expression/blend/2008\"\n\
         \x20       xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"\n\
         \x20       mc:Ignorable=\"d\"{size_attrs}\n\
         \x20       x:Class=\"{ns}.{name}\">\n\
         \x20   <Grid>\n\
         \x20       \n\
         \x20   </Grid>\n\
         </{kind}>\n"
    );

    let cs_content = format!(
        "using Avalonia.Controls;\n\n\
         namespace {ns};\n\n\
         public partial class {name} : {kind}\n\
         {{\n\
         \x20   public {name}()\n\
         \x20   {{\n\
         \x20       InitializeComponent();\n\
         \x20   }}\n\
         }}\n"
    );

    (axaml_content, cs_content)
}

/// Если текущий .axaml файл пустой — предлагает инициализировать его как UserControl или Window
pub fn init_empty_file_actions(uri: &Url, text: &str) -> Vec<CodeAction> {
    if !text.trim().is_empty() {
        return vec![];
    }
    let Ok(axaml_path) = uri.to_file_path() else {
        return vec![];
    };
    let Some(stem) = axaml_path.file_stem().and_then(|s| s.to_str()) else {
        return vec![];
    };
    if !is_ident(stem) {
        return vec![];
    }
    let Some(dir) = axaml_path.parent() else {
        return vec![];
    };
    let ns = infer_namespace(dir);

    let mut out = Vec::new();
    for (kind, title) in [
        ("UserControl", format!("Инициализировать {stem} как Avalonia UserControl")),
        ("Window", format!("Инициализировать {stem} как Avalonia Window")),
    ] {
        let (axaml_content, _) = build_view_texts(&ns, kind, stem);
        let mut changes = HashMap::new();
        changes.insert(
            uri.clone(),
            vec![TextEdit {
                range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                new_text: axaml_content,
            }],
        );
        out.push(CodeAction {
            title,
            kind: Some(CodeActionKind::QUICKFIX),
            is_preferred: Some(kind == "UserControl"),
            edit: Some(WorkspaceEdit {
                changes: Some(changes),
                ..Default::default()
            }),
            command: Some(Command {
                title: "Создать code-behind".to_string(),
                command: INIT_VIEW_CS.to_string(),
                arguments: Some(vec![json!(uri.as_str()), json!(kind), json!(stem)]),
            }),
            ..Default::default()
        });
    }
    out
}

/// Пункты автодополнения для пустого файла (`ax-usercontrol`, `ax-window`)
pub fn init_empty_file_completions(uri: &Url, text: &str) -> Vec<CompletionItem> {
    if text.trim().len() > 16 {
        return vec![];
    }
    let Ok(axaml_path) = uri.to_file_path() else {
        return vec![];
    };
    let Some(stem) = axaml_path.file_stem().and_then(|s| s.to_str()) else {
        return vec![];
    };
    if !is_ident(stem) {
        return vec![];
    }
    let Some(dir) = axaml_path.parent() else {
        return vec![];
    };
    let ns = infer_namespace(dir);
    let end_pos = position_of(text, text.len());

    let mut out = Vec::new();
    for (label, kind, desc) in [
        ("ax-usercontrol", "UserControl", format!("Каркас UserControl для {stem} (+ .axaml.cs)")),
        ("ax-window", "Window", format!("Каркас Window для {stem} (+ .axaml.cs)")),
    ] {
        let (axaml_content, _) = build_view_texts(&ns, kind, stem);
        out.push(CompletionItem {
            label: label.to_string(),
            sort_text: Some(format!("0_{label}")),
            kind: Some(CompletionItemKind::SNIPPET),
            detail: Some(desc),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range: Range::new(Position::new(0, 0), end_pos),
                new_text: axaml_content,
            })),
            command: Some(Command {
                title: "Создать code-behind".to_string(),
                command: INIT_VIEW_CS.to_string(),
                arguments: Some(vec![json!(uri.as_str()), json!(kind), json!(stem)]),
            }),
            ..Default::default()
        });
    }
    out
}

/// Создаёт только `.axaml.cs` рядом с инициализированным `.axaml` (если его ещё нет)
pub fn run_init_view_cs(uri: &Url, kind: &str, name: &str) -> Option<()> {
    if !is_ident(name) || !matches!(kind, "UserControl" | "Window") {
        return None;
    }
    let axaml_path = uri.to_file_path().ok()?;
    let dir = axaml_path.parent()?;
    let ns = infer_namespace(dir);
    let new_cs = dir.join(format!("{name}.axaml.cs"));
    if new_cs.exists() {
        return None;
    }
    let (_, cs_content) = build_view_texts(&ns, kind, name);
    std::fs::write(&new_cs, cs_content).ok()
}
