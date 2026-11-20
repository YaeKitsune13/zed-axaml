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
        "Control instance name for accessing from C# code-behind and bindings.",
    ),
    (
        "x:DataType",
        "Data context type (ViewModel) for {CompiledBinding} expressions.",
    ),
    (
        "x:Class",
        "Full C# class name (Namespace.ClassName) associated with this XAML file.",
    ),
    (
        "x:Key",
        "Unique resource key in a Resources or Styles dictionary.",
    ),
    (
        "x:CompileBindings",
        "Enables or disables compiled bindings by default (True / False).",
    ),
    (
        "x:FieldModifier",
        "Access modifier of the generated C# field (public, internal, private).",
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
        "Data template for rendering collection items (ItemTemplate).",
    ),
    (
        "ControlTemplate",
        "Avalonia.Markup.Xaml.Templates.ControlTemplate",
        "Control visual tree template.",
    ),
    (
        "RowDefinition",
        "Avalonia.Controls.RowDefinition",
        "Grid row definition.",
    ),
    (
        "ColumnDefinition",
        "Avalonia.Controls.ColumnDefinition",
        "Grid column definition.",
    ),
];

pub const TAG_SNIPPETS: &[(&str, &str, &str)] = &[
    (
        "ax-itemtemplate",
        "ListBox.ItemTemplate card snippet with DataTemplate and Border",
        "ListBox.ItemTemplate>\n\t<DataTemplate>\n\t\t<Border BorderBrush=\"Gray\" BorderThickness=\"1\" CornerRadius=\"5\" Padding=\"10\" Margin=\"5\">\n\t\t\t$0\n\t\t</Border>\n\t</DataTemplate>\n</ListBox.ItemTemplate>",
    ),
    (
        "ax-grid",
        "Grid with predefined ColumnDefinitions and RowDefinitions",
        "Grid ColumnDefinitions=\"${1:*, *}\" RowDefinitions=\"${2:Auto, *}\">\n\t$0\n</Grid>",
    ),
    (
        "ax-stack-h",
        "Horizontal StackPanel with spacing",
        "StackPanel Orientation=\"Horizontal\" Spacing=\"${1:10}\">\n\t$0\n</StackPanel>",
    ),
    (
        "ax-dock",
        "DockPanel layout with top header and main content",
        "DockPanel Margin=\"${1:15}\">\n\t<StackPanel DockPanel.Dock=\"Top\" Orientation=\"Horizontal\" Spacing=\"10\" Margin=\"0,0,0,10\">\n\t\t$0\n\t</StackPanel>\n</DockPanel>",
    ),
];

pub const BINDING_SNIPPETS: &[(&str, &str, &str)] = &[
    (
        "{Binding}",
        "Binding to a data context property",
        "{Binding $0}",
    ),
    (
        "{Binding StringFormat}",
        "Binding with StringFormat (e.g. currency)",
        "{Binding ${1:Property}, StringFormat='{}{0}${2: ₽}'}",
    ),
    (
        "{CompiledBinding}",
        "Compiled binding (requires x:DataType)",
        "{CompiledBinding $0}",
    ),
    (
        "{StaticResource}",
        "Reference to a static resource by key",
        "{StaticResource $0}",
    ),
    (
        "{DynamicResource}",
        "Reference to a dynamic resource by key",
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
