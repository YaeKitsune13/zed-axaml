use dotnetdll::prelude::*;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Serialize)]
struct TypeMeta {
    name: String,
    namespace: String,
    base: Option<String>,
    properties: Vec<String>,
    attached: Vec<String>,
    events: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    summary: String,
    /// ключи: "P:Имя" (свойство), "E:Имя" (событие), "A:Имя" (прикреплённое)
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    docs: BTreeMap<String, String>,
}

fn is_public(td: &TypeDefinition<'_>) -> bool {
    td.encloser.is_none() && format!("{:?}", td.flags.accessibility) == "Public"
}

fn base_name(res: &Resolution<'_>, td: &TypeDefinition<'_>) -> Option<String> {
    match td.extends.as_ref()? {
        TypeSource::User(u) => Some(u.type_name(res)),
        TypeSource::Generic { base, .. } => Some(base.type_name(res)),
    }
}

fn own_members(td: &TypeDefinition<'_>) -> (Vec<String>, Vec<String>, Vec<String>) {
    let props: Vec<String> = td
        .properties
        .iter()
        .filter(|p| {
            [&p.getter, &p.setter].iter().any(|m| {
                m.as_ref()
                    .is_some_and(|m| format!("{:?}", m.accessibility) == "Access(Public)")
            })
        })
        .map(|p| p.name.to_string())
        .collect();

    let attached: Vec<String> = td
        .fields
        .iter()
        .filter_map(|f| f.name.strip_suffix("Property"))
        .filter(|stem| !stem.is_empty() && !props.iter().any(|p| p == stem))
        .map(|s| s.to_string())
        .collect();

    let events: Vec<String> = td
        .events
        .iter()
        .filter(|e| format!("{:?}", e.add_listener.accessibility) == "Access(Public)")
        .map(|e| e.name.to_string())
        .collect();

    (props, attached, events)
}

// ---------- документация из XML ----------

fn attr_value<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=\"");
    let s = tag.find(&key)? + key.len();
    let e = tag[s..].find('"')?;
    Some(&tag[s..s + e])
}

/// "P:Avalonia.Controls.Button.Command" -> "Command"
fn short_name(v: &str) -> String {
    let v = if v.len() > 2 && v.as_bytes()[1] == b':' {
        &v[2..]
    } else {
        v
    };
    let v = v.split('(').next().unwrap_or(v);
    let name = v.rsplit('.').next().unwrap_or(v);
    name.split('`').next().unwrap_or(name).to_string()
}

fn clean_doc(raw: &str) -> String {
    let mut out = String::new();
    let mut rest = raw;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        let after = &rest[lt + 1..];
        let Some(gt) = after.find('>') else {
            rest = "";
            break;
        };
        let tag = &after[..gt];
        if tag.starts_with("see ")
            || tag.starts_with("paramref ")
            || tag.starts_with("typeparamref ")
        {
            if let Some(v) = attr_value(tag, "cref")
                .or_else(|| attr_value(tag, "langword"))
                .or_else(|| attr_value(tag, "name"))
            {
                out.push_str(&short_name(v));
            }
        } else if tag.starts_with("para") || tag.starts_with("/para") || tag.starts_with("br") {
            out.push(' ');
        }
        rest = &after[gt + 1..];
    }
    out.push_str(rest);

    let s = out
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// все *.xml в папке: "P:Namespace.Type.Member" -> текст из <summary>
fn load_docs(dir: &Path) -> HashMap<String, String> {
    let mut docs = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return docs;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.extension().is_some_and(|x| x == "xml") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut rest = text.as_str();
        while let Some(i) = rest.find("<member name=\"") {
            rest = &rest[i + 14..];
            let Some(q) = rest.find('"') else { break };
            let Some(end) = rest.find("</member>") else {
                break;
            };
            if q < end {
                let id = &rest[..q];
                let body = &rest[q..end];
                if let Some(s) = body.find("<summary>") {
                    if let Some(e) = body[s..].find("</summary>") {
                        let t = clean_doc(&body[s + 9..s + e]);
                        if !t.is_empty() {
                            docs.insert(id.to_string(), t);
                        }
                    }
                }
            }
            rest = &rest[end + 9..];
        }
    }
    docs
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(
        args.next()
            .expect("использование: axaml-meta <папка с dll и xml> [полное имя типа]"),
    );
    let only = args.next();

    let docs = load_docs(&dir);

    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("не удалось открыть папку")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "dll"))
        .collect();
    paths.sort();

    // байты должны жить дольше, чем Resolution, который на них ссылается
    let blobs: Vec<Vec<u8>> = paths
        .iter()
        .map(|p| std::fs::read(p).expect("не удалось прочитать dll"))
        .collect();

    let resolutions: Vec<Resolution> = blobs
        .iter()
        .filter_map(|b| Resolution::parse(b, ReadOptions::default()).ok())
        .collect();

    let mut types: BTreeMap<String, TypeMeta> = BTreeMap::new();
    for res in &resolutions {
        for td in &res.type_definitions {
            if !is_public(td) {
                continue;
            }
            let full = td.type_name();
            let (properties, attached, events) = own_members(td);

            let mut member_docs = BTreeMap::new();
            for p in &properties {
                if let Some(d) = docs.get(&format!("P:{full}.{p}")) {
                    member_docs.insert(format!("P:{p}"), d.clone());
                }
            }
            for e in &events {
                if let Some(d) = docs.get(&format!("E:{full}.{e}")) {
                    member_docs.insert(format!("E:{e}"), d.clone());
                }
            }
            for a in &attached {
                if let Some(d) = docs.get(&format!("F:{full}.{a}Property")) {
                    member_docs.insert(format!("A:{a}"), d.clone());
                }
            }

            types.insert(
                full.clone(),
                TypeMeta {
                    name: td.name.to_string(),
                    namespace: td.namespace.as_deref().unwrap_or("").to_string(),
                    base: base_name(res, td),
                    properties,
                    attached,
                    events,
                    summary: docs.get(&format!("T:{full}")).cloned().unwrap_or_default(),
                    docs: member_docs,
                },
            );
        }
    }

    match only {
        // отладка: развернуть цепочку наследования одного типа
        Some(name) => {
            let mut cur = Some(name);
            let mut seen = HashSet::new();
            while let Some(n) = cur {
                if !seen.insert(n.clone()) {
                    break;
                }
                match types.get(&n) {
                    Some(t) => {
                        println!("{n}");
                        println!("  properties: {}", t.properties.join(", "));
                        println!("  attached:   {}", t.attached.join(", "));
                        println!("  events:     {}", t.events.join(", "));
                        cur = t.base.clone();
                    }
                    None => {
                        println!("{n}  (вне загруженных сборок)");
                        break;
                    }
                }
            }
        }
        // обычный режим: весь индекс в JSON
        None => {
            println!("{}", serde_json::to_string(&types).unwrap());
        }
    }
}
