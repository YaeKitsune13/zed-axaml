use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct TypeMeta {
    pub name: String,
    #[allow(dead_code)]
    pub namespace: String,
    pub base: Option<String>,
    pub properties: Vec<String>,
    pub attached: Vec<String>,
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub docs: HashMap<String, String>,
}

pub const X_DIRECTIVES: &[(&str, &str)] = &[
    (
        "x:Name",
        "Имя экземпляра контрола для обращения из C# кода (code-behind) и привязок.",
    ),
    (
        "x:DataType",
        "Тип контекста данных (ViewModel) для компилируемых привязок {CompiledBinding}.",
    ),
    (
        "x:Class",
        "Полное имя C# класса (Namespace.ClassName), связанного с этим XAML-файлом.",
    ),
    (
        "x:Key",
        "Уникальный ключ ресурса в словаре Resources или Styles.",
    ),
    (
        "x:CompileBindings",
        "Включает или отключает компилируемые привязки по умолчанию (True / False).",
    ),
    (
        "x:FieldModifier",
        "Модификатор доступа сгенерированного поля в C# (public, internal, private).",
    ),
];

pub const TAG_BASES: &[&str] = &[
    "Avalonia.Controls.Control",
    "Avalonia.Application",
    "Avalonia.Styling.StyleBase",
    "Avalonia.Styling.SetterBase",
    "Avalonia.Controls.DefinitionBase",
    "Avalonia.Animation.KeyFrame",
    "Avalonia.Animation.Animation",
    "Avalonia.Controls.Templates.IDataTemplate",
];
