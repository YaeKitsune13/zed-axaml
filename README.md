# zed-axaml

Avalonia XAML (`.axaml`) language support and Language Server Protocol (`axaml-ls`) integration for the [Zed editor](https://zed.dev/).

`axaml-ls` reads metadata extracted from compiled Avalonia assemblies along with your project's `.cs` and `.axaml` files to provide autocompletion, code navigation, diagnostics, and code-behind generation.

## Features

- **Tag & Property Completion:** Autocompletes Avalonia controls, dot-notation property elements (`<Grid.RowDefinitions>`, `<Window.Styles>`), attached properties (`Grid.Row`), and XAML directives (`x:Name`, `x:DataType`, `x:Class`, `x:Key`, `x:CompileBindings`).
- **Styles & Setters:** Suggests control types and pseudo-classes (`:pointerover`, `:pressed`, `:focus`, etc.) inside `<Style Selector="...">`, and resolves target control properties and enum values inside `<Setter Property="..." Value="..." />`.
- **Bindings & ViewModel Support:** Resolves `x:DataType` across the element tree to autocomplete properties and `[RelayCommand]` commands inside `{Binding}` and `{CompiledBinding}`, with diagnostics for invalid compiled binding paths.
- **Resources & Navigation (`Go to Definition`):** Autocompletes `{StaticResource}` / `{DynamicResource}` keys from the current file and `App.axaml`, and jumps to resource definitions, `x:Class` declarations, event handlers in `.axaml.cs`, and custom controls.
- **Code-Behind Generation & Code Actions:** Generates event handler stubs in `.axaml.cs` with matching `*EventArgs` signatures and `using` directives, initializes empty `.axaml` files as `UserControl` or `Window`, and syncs `x:Name` / `Name` fields for C# tooling.
- **Hover Documentation & Formatting:** Shows C# XML summaries on hover, highlights properties and events via semantic tokens, and formats `.axaml` documents.

## Repository Structure

- **`src/` & `languages/axaml/`** — Zed extension entry point (auto-downloads `axaml-ls` and `meta.json` from GitHub Releases if not present in `PATH`) and Tree-sitter syntax queries.
- **`axaml-ls/`** — Standalone LSP server written in Rust (`tower-lsp`).
- **`axaml-meta/`** — CLI utility that extracts type metadata and XML documentation from Avalonia `.dll` and `.xml` files into `meta.json`.

## Installation

### Via Zed Extensions

Install **AXAML** directly from the Zed Extensions marketplace. On first launch, the extension automatically downloads the `axaml-ls` binary and default `meta.json` for your platform.

### Building from Source

If you want to build and use a local version (the extension prioritizes `axaml-ls` found in `PATH`):

```bash
git clone [https://github.com/YaeKitsune13/zed-axaml.git](https://github.com/YaeKitsune13/zed-axaml.git)
cd zed-axaml/axaml-ls
cargo build --release
install -Dm755 target/release/axaml-ls ~/.local/bin/axaml-ls
```
