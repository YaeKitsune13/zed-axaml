use dotnetdll::prelude::*;

fn main() -> Result<(), DLLError> {
    let path = std::env::args()
        .nth(1)
        .expect("использование: axaml-meta <путь к dll>");

    let bytes = std::fs::read(&path).expect("не удалось прочитать файл");
    let res = Resolution::parse(&bytes, ReadOptions::default())?;

    for (_, typedef) in res.enumerate_type_definitions() {
        // только публичные типы верхнего уровня
        if typedef.encloser.is_some() || format!("{:?}", typedef.flags.accessibility) != "Public" {
            continue;
        }

        let base = match &typedef.extends {
            Some(TypeSource::User(u)) => u.type_name(&res),
            Some(TypeSource::Generic { base, .. }) => base.type_name(&res),
            None => "-".to_string(),
        };

        println!(
            "{} : {}  (fields: {}, properties: {})",
            typedef.type_name(),
            base,
            typedef.fields.len(),
            typedef.properties.len()
        );
    }

    Ok(())
}
