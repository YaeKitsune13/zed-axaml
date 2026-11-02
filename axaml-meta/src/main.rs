use dotnetdll::prelude::*;
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

#[derive(Serialize)]
struct TypeMeta {
    name: String,
    namespace: String,
    base: Option<String>,
    properties: Vec<String>,
    attached: Vec<String>,
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

fn own_members(td: &TypeDefinition<'_>) -> (Vec<String>, Vec<String>) {
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

    (props, attached)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(
        args.next()
            .expect("использование: axaml-meta <папка с dll> [полное имя типа]"),
    );
    let only = args.next();

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
            let (properties, attached) = own_members(td);
            types.insert(
                td.type_name(),
                TypeMeta {
                    name: td.name.to_string(),
                    namespace: td.namespace.as_deref().unwrap_or("").to_string(),
                    base: base_name(res, td),
                    properties,
                    attached,
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
