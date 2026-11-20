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
    #[serde(default)]
    pub event_args: HashMap<String, String>,
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
    "Avalonia.Markup.Xaml.Templates.DataTemplate",
    "Avalonia.Markup.Xaml.Templates.ControlTemplate",
];

pub const EXTRA_TAGS: &[(&str, &str, &str)] = &[
    (
        "DataTemplate",
        "Avalonia.Markup.Xaml.Templates.DataTemplate",
        "Шаблон отображения данных для элементов коллекции (ItemTemplate).",
    ),
    (
        "ControlTemplate",
        "Avalonia.Markup.Xaml.Templates.ControlTemplate",
        "Шаблон визуального дерева контрола.",
    ),
    (
        "RowDefinition",
        "Avalonia.Controls.RowDefinition",
        "Определение строки в Grid.",
    ),
    (
        "ColumnDefinition",
        "Avalonia.Controls.ColumnDefinition",
        "Определение столбца в Grid.",
    ),
];

pub const TAG_SNIPPETS: &[(&str, &str, &str)] = &[
    (
        "ax-itemtemplate",
        "Шаблон карточки ListBox.ItemTemplate с DataTemplate и Border",
        "ListBox.ItemTemplate>\n\t<DataTemplate>\n\t\t<Border BorderBrush=\"Gray\" BorderThickness=\"1\" CornerRadius=\"5\" Padding=\"10\" Margin=\"5\">\n\t\t\t$0\n\t\t</Border>\n\t</DataTemplate>\n</ListBox.ItemTemplate>",
    ),
    (
        "ax-grid",
        "Grid с заданными ColumnDefinitions и RowDefinitions",
        "Grid ColumnDefinitions=\"${1:*, *}\" RowDefinitions=\"${2:Auto, *}\">\n\t$0\n</Grid>",
    ),
    (
        "ax-stack-h",
        "Горизонтальный StackPanel с отступами",
        "StackPanel Orientation=\"Horizontal\" Spacing=\"${1:10}\">\n\t$0\n</StackPanel>",
    ),
    (
        "ax-dock",
        "Каркас DockPanel с верхней шапкой и основным контентом",
        "DockPanel Margin=\"${1:15}\">\n\t<StackPanel DockPanel.Dock=\"Top\" Orientation=\"Horizontal\" Spacing=\"10\" Margin=\"0,0,0,10\">\n\t\t$0\n\t</StackPanel>\n</DockPanel>",
    ),
];

pub const BINDING_SNIPPETS: &[(&str, &str, &str)] = &[
    (
        "{Binding}",
        "Привязка к свойству контекста данных",
        "{Binding $0}",
    ),
    (
        "{Binding StringFormat}",
        "Привязка с форматированием строки (например, цена в ₽)",
        "{Binding ${1:Property}, StringFormat='{}{0}${2: ₽}'}",
    ),
    (
        "{CompiledBinding}",
        "Компилируемая привязка (требует x:DataType)",
        "{CompiledBinding $0}",
    ),
    (
        "{StaticResource}",
        "Ссылка на статический ресурс по ключу",
        "{StaticResource $0}",
    ),
    (
        "{DynamicResource}",
        "Ссылка на динамический ресурс по ключу",
        "{DynamicResource $0}",
    ),
];

pub fn enum_values_for_attr(attr: &str) -> &'static [&'static str] {
    match attr {
        "Orientation" => &["Vertical", "Horizontal"],
        "HorizontalAlignment" | "HorizontalContentAlignment" => {
            &["Left", "Center", "Right", "Stretch"]
        }
        "VerticalAlignment" | "VerticalContentAlignment" => &["Top", "Center", "Bottom", "Stretch"],
        "DockPanel.Dock" | "Dock" => &["Top", "Bottom", "Left", "Right"],
        "TextWrapping" => &["Wrap", "NoWrap", "WrapWithOverflow"],
        "TextAlignment" => &["Left", "Center", "Right", "Justify"],
        "FontWeight" => &["Normal", "Bold", "SemiBold", "Light", "Medium", "Thin"],
        "FontStyle" => &["Normal", "Italic", "Oblique"],
        "WindowStartupLocation" => &["CenterScreen", "CenterOwner", "Manual"],
        "Stretch" => &["Uniform", "UniformToFill", "Fill", "None"],
        "SelectionMode" => &["Single", "Multiple", "Toggle", "AlwaysSelected"],
        "ScrollBarVisibility"
        | "ScrollViewer.VerticalScrollBarVisibility"
        | "ScrollViewer.HorizontalScrollBarVisibility" => {
            &["Auto", "Visible", "Hidden", "Disabled"]
        }
        "IsVisible"
        | "IsEnabled"
        | "IsReadOnly"
        | "IsChecked"
        | "IsSelected"
        | "IsHitTestVisible"
        | "AutoGenerateColumns"
        | "CanUserResizeColumns"
        | "CanUserSortColumns"
        | "CanUserReorderColumns"
        | "x:CompileBindings" => &["True", "False"],
        _ => &[],
    }
}
