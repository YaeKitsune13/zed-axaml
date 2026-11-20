use crate::parser::{byte_range_to_lsp, scan, Kind};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::*;

#[derive(Clone, Debug)]
pub struct CsMember {
    pub name: String,
    pub member_type: String,
    pub is_command: bool,
}

#[derive(Clone, Debug)]
pub struct CsClass {
    pub name: String,
    pub namespace: String,
    pub base_type: Option<String>,
    pub members: Vec<CsMember>,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct ResourceItem {
    pub key: String,
    pub kind: String,
    pub uri: Url,
    pub range: Range,
}

#[derive(Default, Clone)]
pub struct WorkspaceData {
    pub classes: Vec<CsClass>,
    pub resources: Vec<ResourceItem>,
}

impl WorkspaceData {
    pub fn scan_for_file(uri: &Url, current_text: &str) -> Self {
        let mut data = Self::default();
        let Ok(file_path) = uri.to_file_path() else {
            return data;
        };
        let root = find_project_root(&file_path)
            .unwrap_or_else(|| file_path.parent().unwrap_or(Path::new(".")).to_path_buf());

        let mut cs_files = Vec::new();
        let mut axaml_files = Vec::new();
        collect_files(&root, &mut cs_files, &mut axaml_files, 0);

        for cs in cs_files {
            if let Ok(content) = fs::read_to_string(&cs) {
                data.classes.extend(parse_cs_classes(&content, &cs));
            }
        }

        // Ресурсы из текущего буфера (учитывая несохранённые правки)
        data.resources.extend(parse_axaml_resources(current_text, uri));

        // Ресурсы из App.axaml и словарей ресурсов проекта
        for ax in axaml_files {
            if ax == file_path {
                continue;
            }
            let fname = ax.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if fname == "App.axaml" || fname.contains("Resource") || fname.contains("Style") || fname.contains("Theme") {
                if let (Ok(content), Ok(ax_uri)) = (fs::read_to_string(&ax), Url::from_file_path(&ax)) {
                    data.resources.extend(parse_axaml_resources(&content, &ax_uri));
                }
            }
        }

        data
    }

    pub fn find_class(&self, ns: &str, name: &str) -> Option<&CsClass> {
        self.classes
            .iter()
            .find(|c| c.name == name && (ns.is_empty() || c.namespace == ns))
            .or_else(|| self.classes.iter().find(|c| c.name == name))
    }

    pub fn is_control_class(&self, ns: &str, name: &str) -> bool {
        let mut cur_ns = ns.to_string();
        let mut cur_name = name.to_string();
        for _ in 0..16 {
            let Some(cls) = self.find_class(&cur_ns, &cur_name) else {
                break;
            };
            let Some(base) = &cls.base_type else {
                return false;
            };
            let base_short = base.rsplit('.').next().unwrap_or(base);
            if matches!(
                base_short,
                "Control" | "UserControl" | "Window" | "TemplatedControl" | "Panel" | "Decorator" | "ContentControl" | "ItemsControl"
            ) {
                return true;
            }
            cur_name = base_short.to_string();
            cur_ns.clear();
        }
        false
    }

    /// Разрешает цепочку свойств (например, `SelectedUser.Name`) начиная с заданного типа
    pub fn resolve_member_chain(&self, start_ns: &str, start_type: &str, prefix_path: &str) -> Vec<CsMember> {
        let parts: Vec<&str> = prefix_path.split('.').collect();
        let mut cur_ns = start_ns.to_string();
        let mut cur_type = start_type.to_string();

        for part in &parts[..parts.len().saturating_sub(1)] {
            let Some(cls) = self.find_class(&cur_ns, &cur_type) else {
                return vec![];
            };
            let Some(mem) = cls.members.iter().find(|m| m.name == *part) else {
                return vec![];
            };
            cur_type = clean_type_name(&mem.member_type);
            cur_ns.clear();
        }

        let Some(cls) = self.find_class(&cur_ns, &cur_type) else {
            return vec![];
        };
        cls.members.clone()
    }
}

fn find_project_root(start: &Path) -> Option<PathBuf> {
    let mut cur = start.parent()?;
    for _ in 0..10 {
        if let Ok(entries) = fs::read_dir(cur) {
            if entries.filter_map(|e| e.ok()).any(|e| {
                e.path()
                    .extension()
                    .is_some_and(|ext| ext == "csproj" || ext == "sln")
            }) {
                return Some(cur.to_path_buf());
            }
        }
        cur = cur.parent()?;
    }
    None
}

fn collect_files(dir: &Path, cs: &mut Vec<PathBuf>, axaml: &mut Vec<PathBuf>, depth: usize) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_str().unwrap_or("");
        if matches!(name_str, "bin" | "obj" | ".git" | ".vs" | ".idea" | "node_modules") {
            continue;
        }
        if path.is_dir() {
            collect_files(&path, cs, axaml, depth + 1);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if ext == "cs" && !name_str.ends_with(".g.cs") {
                cs.push(path);
            } else if ext == "axaml" || ext == "xaml" {
                axaml.push(path);
            }
        }
    }
}

fn clean_type_name(raw: &str) -> String {
    let t = raw.trim().trim_end_matches('?');
    if let Some((_, inner)) = t.split_once('<') {
        inner.trim_end_matches('>').trim().trim_end_matches('?').to_string()
    } else {
        t.rsplit('.').next().unwrap_or(t).to_string()
    }
}

fn parse_cs_classes(text: &str, path: &Path) -> Vec<CsClass> {
    let mut classes = Vec::new();
    let mut namespace = String::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("namespace ") {
            namespace = rest
                .trim_end_matches(';')
                .trim_end_matches('{')
                .trim()
                .to_string();
            break;
        }
    }

    let bytes = text.as_bytes();
    let mut pos = 0;
    while let Some(idx) = text[pos..].find("class ") {
        let abs = pos + idx;
        let prev_ok = abs == 0 || bytes[abs - 1].is_ascii_whitespace();
        let after_kw = abs + 6;
        if !prev_ok || after_kw >= text.len() {
            pos = after_kw;
            continue;
        }
        let header_end = text[after_kw..].find('{').map(|o| after_kw + o).unwrap_or(text.len());
        let header = text[after_kw..header_end].trim();
        let mut parts = header.split(':');
        let name_part = parts.next().unwrap_or("").trim();
        let name = name_part
            .split(|c: char| c.is_whitespace() || c == '<' || c == '(')
            .next()
            .unwrap_or("")
            .to_string();

        if name.is_empty() || !name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') {
            pos = header_end;
            continue;
        }

        let base_type = parts
            .next()
            .and_then(|b| b.split(',').next())
            .map(|b| b.trim().split('<').next().unwrap_or("").trim().to_string())
            .filter(|b| !b.is_empty());

        let body_end = find_matching_brace(text, header_end).unwrap_or(text.len());
        let body = &text[header_end..body_end];
        let members = parse_cs_members(body);

        classes.push(CsClass {
            name,
            namespace: namespace.clone(),
            base_type,
            members,
            path: path.to_path_buf(),
        });
        pos = body_end;
    }

    classes
}

fn find_matching_brace(text: &str, open: usize) -> Option<usize> {
    let b = text.as_bytes();
    if b.get(open) != Some(&b'{') {
        return None;
    }
    let mut depth = 0i32;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn parse_cs_members(body: &str) -> Vec<CsMember> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut relay_pending = false;

    for line in body.lines() {
        let t = line.trim();
        if t.starts_with("[RelayCommand") {
            relay_pending = true;
            continue;
        }
        if t.starts_with('[') {
            continue;
        }

        if let Some(rest) = t.strip_prefix("public ") {
            let rest = rest
                .trim_start_matches("virtual ")
                .trim_start_matches("override ")
                .trim_start_matches("async ")
                .trim_start_matches("Task ")
                .trim_start_matches("void ");

            // Метод с [RelayCommand] -> генерирует свойство <MethodName>Command
            if relay_pending {
                relay_pending = false;
                if let Some(paren) = rest.find('(') {
                    let mname = rest[..paren].split_whitespace().last().unwrap_or("");
                    if !mname.is_empty() {
                        let cmd_name = format!("{mname}Command");
                        if seen.insert(cmd_name.clone()) {
                            out.push(CsMember {
                                name: cmd_name,
                                member_type: "IRelayCommand".to_string(),
                                is_command: true,
                            });
                        }
                    }
                }
                continue;
            }

            // Публичное свойство: `public Type Name { get; ... }` или `public Type Name => ...`
            let decl = rest
                .split('{')
                .next()
                .unwrap_or("")
                .split("=>")
                .next()
                .unwrap_or("")
                .trim();
            if !decl.contains('(') {
                let mut tokens = decl.split_whitespace().rev();
                if let (Some(prop_name), Some(prop_type)) = (tokens.next(), tokens.next()) {
                    let clean_name = prop_name.trim_end_matches(';');
                    if clean_name
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_alphabetic() || c == '_')
                        && seen.insert(clean_name.to_string())
                    {
                        let is_cmd = prop_type.contains("Command");
                        out.push(CsMember {
                            name: clean_name.to_string(),
                            member_type: prop_type.to_string(),
                            is_command: is_cmd,
                        });
                    }
                }
            }
        } else if !t.is_empty() {
            relay_pending = false;
        }
    }

    out
}

fn parse_axaml_resources(text: &str, uri: &Url) -> Vec<ResourceItem> {
    let mut out = Vec::new();
    for tag in scan(text) {
        if tag.kind == Kind::Close {
            continue;
        }
        for attr in &tag.attrs {
            if attr.name == "x:Key" {
                if let (Some(val), Some(off)) = (&attr.value, attr.value_off) {
                    if !val.is_empty() {
                        out.push(ResourceItem {
                            key: val.clone(),
                            kind: tag.name.clone(),
                            uri: uri.clone(),
                            range: byte_range_to_lsp(text, off, off + val.len()),
                        });
                    }
                }
            }
        }
    }
    out
}

/// Извлекает карту префиксов xmlns из корневого тега: `vm` -> `MyApp.ViewModels`
pub fn parse_xmlns_map(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for tag in scan(text) {
        if tag.kind == Kind::Close {
            continue;
        }
        for attr in &tag.attrs {
            if let Some(prefix) = attr.name.strip_prefix("xmlns:") {
                if let Some(val) = &attr.value {
                    let ns = val
                        .strip_prefix("using:")
                        .or_else(|| val.strip_prefix("clr-namespace:"))
                        .unwrap_or(val)
                        .split(';')
                        .next()
                        .unwrap_or("")
                        .to_string();
                    map.insert(prefix.to_string(), ns);
                }
            }
        }
        break;
    }
    map
}

/// Находит активный `x:DataType` для позиции `off` (в текущем теге или вверх по родителям)
pub fn active_data_type(text: &str, off: usize) -> Option<String> {
    let mut stack: Vec<Option<String>> = Vec::new();
    for tag in scan(text) {
        if tag.name_off > off {
            break;
        }
        let dt = tag
            .attrs
            .iter()
            .find(|a| a.name == "x:DataType")
            .and_then(|a| a.value.clone());

        match tag.kind {
            Kind::Open => stack.push(dt),
            Kind::SelfClose => {
                if dt.is_some() {
                    // Проверяем, находится ли курсор внутри этого самозакрывающегося тега
                    if let Some(gt) = text[tag.name_off..].find('>') {
                        if off <= tag.name_off + gt {
                            return dt;
                        }
                    }
                }
            }
            Kind::Close => {
                stack.pop();
            }
        }
    }
    stack.into_iter().rev().flatten().next()
}

/// Извлекает выражение внутри `{...}` под курсором или из значения атрибута
fn extract_markup_call(val: &str) -> Option<(&str, &str)> {
    let inner = val.trim().strip_prefix('{')?.strip_suffix('}')?.trim();
    let (kind, rest) = inner.split_once(char::is_whitespace).unwrap_or((inner, ""));
    let first_arg = rest.split(',').next().unwrap_or("").trim();
    let path = first_arg.strip_prefix("Path=").unwrap_or(first_arg).trim();
    Some((kind, path))
}

impl WorkspaceData {
    /// Подсказки внутри `{Binding ...}`, `{CompiledBinding ...}`, `{StaticResource ...}`, `{DynamicResource ...}`
    pub fn complete_attr_value(&self, text: &str, off: usize, prefix: &str, replace: Range) -> Vec<CompletionItem> {
        let trimmed = prefix.trim_start();
        let Some(after_brace) = trimmed.strip_prefix('{') else {
            return vec![];
        };

        // 1. {StaticResource ...} / {DynamicResource ...}
        for kw in ["StaticResource", "DynamicResource"] {
            if let Some(rest) = after_brace.strip_prefix(kw) {
                if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                    let key_prefix = rest.trim_start();
                    let mut items = Vec::new();
                    for res in &self.resources {
                        if !res.key.starts_with(key_prefix) {
                            continue;
                        }
                        items.push(CompletionItem {
                            label: res.key.clone(),
                            sort_text: Some(format!("0_{}", res.key)),
                            kind: Some(CompletionItemKind::COLOR),
                            detail: Some(format!("Resource ({})", res.kind)),
                            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                                range: replace,
                                new_text: format!("{{{kw} {}}}", res.key),
                            })),
                            ..Default::default()
                        });
                    }
                    return items;
                }
            }
        }

        // 2. {Binding ...} / {CompiledBinding ...}
        for kw in ["Binding", "CompiledBinding"] {
            if let Some(rest) = after_brace.strip_prefix(kw) {
                if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                    let Some(dt_raw) = active_data_type(text, off) else {
                        return vec![];
                    };
                    let xmlns = parse_xmlns_map(text);
                    let (ns, cls_name) = resolve_xml_type(&dt_raw, &xmlns);
                    let expr = rest.trim_start();
                    // Если уже есть запятая (например `{Binding Name, Mode=`), не мешаем
                    if expr.contains(',') {
                        return vec![];
                    }
                    let path_part = expr.strip_prefix("Path=").unwrap_or(expr);
                    let members = self.resolve_member_chain(&ns, &cls_name, path_part);
                    let base_chain = match path_part.rfind('.') {
                        Some(dot) => &path_part[..=dot],
                        None => "",
                    };

                    return members
                        .into_iter()
                        .map(|m| {
                            let full_path = format!("{base_chain}{}", m.name);
                            CompletionItem {
                                label: m.name.clone(),
                                sort_text: Some(format!("0_{}", m.name)),
                                kind: Some(if m.is_command {
                                    CompletionItemKind::METHOD
                                } else {
                                    CompletionItemKind::PROPERTY
                                }),
                                detail: Some(format!("{cls_name}.{} ({})", m.name, m.member_type)),
                                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                                    range: replace,
                                    new_text: format!("{{{kw} {full_path}}}"),
                                })),
                                ..Default::default()
                            }
                        })
                        .collect();
                }
            }
        }

        vec![]
    }

    /// Переход к определению ресурса из `{StaticResource Key}` / `{DynamicResource Key}`
    pub fn goto_resource(&self, text: &str, off: usize) -> Option<Location> {
        for tag in scan(text) {
            if tag.kind == Kind::Close {
                continue;
            }
            for attr in &tag.attrs {
                let (Some(val), Some(val_off)) = (&attr.value, attr.value_off) else {
                    continue;
                };
                if off < val_off || off > val_off + val.len() {
                    continue;
                }
                let (kind, arg) = extract_markup_call(val)?;
                if matches!(kind, "StaticResource" | "DynamicResource") && !arg.is_empty() {
                    if let Some(res) = self.resources.iter().find(|r| r.key == arg) {
                        return Some(Location {
                            uri: res.uri.clone(),
                            range: res.range,
                        });
                    }
                }
            }
        }
        None
    }

    /// Диагностика `{CompiledBinding ...}` по активному `x:DataType`
    pub fn binding_diagnostics(&self, text: &str) -> Vec<Diagnostic> {
        let mut diags = Vec::new();
        let xmlns = parse_xmlns_map(text);

        for tag in scan(text) {
            if tag.kind == Kind::Close {
                continue;
            }
            for attr in &tag.attrs {
                let (Some(val), Some(val_off)) = (&attr.value, attr.value_off) else {
                    continue;
                };
                let Some((kind, path)) = extract_markup_call(val) else {
                    continue;
                };
                if kind != "CompiledBinding" || path.is_empty() || path.starts_with('.') || path.starts_with('#') {
                    continue;
                }
                let Some(dt_raw) = active_data_type(text, attr.name_off) else {
                    continue;
                };
                let (ns, cls_name) = resolve_xml_type(&dt_raw, &xmlns);
                if self.find_class(&ns, &cls_name).is_none() {
                    continue;
                }
                let leaf = path.rsplit('.').next().unwrap_or(path);
                let members = self.resolve_member_chain(&ns, &cls_name, path);
                if !members.iter().any(|m| m.name == leaf) {
                    diags.push(Diagnostic {
                        range: byte_range_to_lsp(text, val_off, val_off + val.len()),
                        severity: Some(DiagnosticSeverity::WARNING),
                        source: Some("axaml-ls".to_string()),
                        message: format!("Property or command '{path}' not found on {cls_name}"),
                        ..Default::default()
                    });
                }
            }
        }
        diags
    }
}

pub fn resolve_xml_type(raw: &str, xmlns: &HashMap<String, String>) -> (String, String) {
    if let Some((prefix, name)) = raw.split_once(':') {
        let ns = xmlns.get(prefix).cloned().unwrap_or_default();
        (ns, name.to_string())
    } else if let Some((ns, name)) = raw.rsplit_once('.') {
        (ns.to_string(), name.to_string())
    } else {
        (String::new(), raw.to_string())
    }
}

impl WorkspaceData {
    /// Переход к определению C#-класса из x:DataType или тега <local:MyControl>
    pub fn goto_type(&self, text: &str, off: usize) -> Option<Location> {
        let xmlns = parse_xmlns_map(text);
        for tag in scan(text) {
            if tag.kind == Kind::Close {
                continue;
            }
            if tag.name.contains(':') && off >= tag.name_off && off <= tag.name_off + tag.name.len() {
                let (ns, cls_name) = resolve_xml_type(&tag.name, &xmlns);
                if let Some(cls) = self.find_class(&ns, &cls_name) {
                    if let (Ok(content), Ok(uri)) = (fs::read_to_string(&cls.path), Url::from_file_path(&cls.path)) {
                        let pat = format!("class {}", cls.name);
                        let start = content.find(&pat).map(|i| i + 6).unwrap_or(0);
                        return Some(Location {
                            uri,
                            range: byte_range_to_lsp(&content, start, start + cls.name.len()),
                        });
                    }
                }
            }
            for attr in &tag.attrs {
                if attr.name == "x:DataType" {
                    if let (Some(val), Some(val_off)) = (&attr.value, attr.value_off) {
                        if off >= val_off && off <= val_off + val.len() {
                            let (ns, cls_name) = resolve_xml_type(val, &xmlns);
                            if let Some(cls) = self.find_class(&ns, &cls_name) {
                                if let (Ok(content), Ok(uri)) = (fs::read_to_string(&cls.path), Url::from_file_path(&cls.path)) {
                                    let pat = format!("class {}", cls.name);
                                    let start = content.find(&pat).map(|i| i + 6).unwrap_or(0);
                                    return Some(Location {
                                        uri,
                                        range: byte_range_to_lsp(&content, start, start + cls.name.len()),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }
}

/// Генерирует/обновляет файл в `obj/axaml-ls.generated.g.cs`, чтобы C# LSP сразу видел поля из x:Name / Name без ручной сборки
pub fn sync_named_controls_g_cs(uri: &Url, text: &str) {
    let Ok(axaml_path) = uri.to_file_path() else {
        return;
    };
    let Some(root) = find_project_root(&axaml_path) else {
        return;
    };

    // Извлекаем x:Class="Namespace.ClassName"
    let mut full_class = None;
    let mut named: Vec<(String, String)> = Vec::new();
    let mut seen = HashSet::new();

    for tag in scan(text) {
        if tag.kind == Kind::Close {
            continue;
        }
        if full_class.is_none() {
            if let Some(cls) = tag.attrs.iter().find(|a| a.name == "x:Class").and_then(|a| a.value.clone()) {
                full_class = Some(cls);
            }
        }
        let elem_name = tag
            .attrs
            .iter()
            .find(|a| a.name == "x:Name" || a.name == "Name")
            .and_then(|a| a.value.as_deref())
            .map(str::trim)
            .filter(|n| !n.is_empty());

        if let Some(n) = elem_name {
            if seen.insert(n.to_string()) {
                let type_name = tag.name.rsplit(':').next().unwrap_or(&tag.name).to_string();
                named.push((type_name, n.to_string()));
            }
        }
    }

    let obj_dir = root.join("obj");
    if !obj_dir.exists() {
        return;
    }
    let gen_file = obj_dir.join("axaml-ls.generated.g.cs");

    let Some(full_class) = full_class else {
        return;
    };
    let (ns, cls_name) = match full_class.rsplit_once('.') {
        Some((n, c)) => (n, c),
        None => ("", full_class.as_str()),
    };

    // Проверяем, не сгенерировал ли уже Avalonia.Generators эти поля в obj/
    let mut all_present_in_official = !named.is_empty();
    let mut stack = vec![obj_dir.clone()];
    let mut official_content = String::new();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for e in entries.filter_map(|e| e.ok()) {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p != gen_file && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(".g.cs")) {
                if let Ok(c) = fs::read_to_string(&p) {
                    if c.contains(&format!("class {cls_name}")) {
                        official_content.push_str(&c);
                    }
                }
            }
        }
    }

    if official_content.is_empty() {
        all_present_in_official = false;
    } else {
        for (_, name) in &named {
            if !official_content.contains(name) {
                all_present_in_official = false;
                break;
            }
        }
    }

    if named.is_empty() || all_present_in_official {
        let _ = fs::remove_file(&gen_file);
        return;
    }

    // Фильтруем только те имена, которых ещё нет в официальном .g.cs
    let missing: Vec<(String, String)> = named
        .into_iter()
        .filter(|(_, name)| !official_content.contains(name))
        .collect();

    if missing.is_empty() {
        let _ = fs::remove_file(&gen_file);
        return;
    }

    let mut code = String::from("// <auto-generated by axaml-ls/>
using Avalonia.Controls;

");
    if !ns.is_empty() {
        code.push_str(&format!("namespace {ns};

"));
    }
    code.push_str(&format!("partial class {cls_name}
{{
"));
    for (ty, name) in missing {
        code.push_str(&format!(
            "    internal {ty} {name} => this.FindControl<{ty}>(\"{name}\")!;
"
        ));
    }
    code.push_str("}
");

    let _ = fs::write(&gen_file, code);
}
