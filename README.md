# zed-axaml

A lightweight, high-performance Language Server Protocol (LSP) implementation tailored for editing **Avalonia XAML (`.axaml`)** files inside the [Zed editor](https://zed.dev/).

Built with Rust and `tower-lsp`, `axaml-ls` parses metadata directly from compiled Avalonia and .NET assemblies to provide native autocompletion, semantic highlighting, and documentation hovers without the heavy overhead of OmniSharp or full IDE stacks.

---

## Features

- **Smart Tag Completion:** Autocompletes Avalonia controls (`Button`, `TextBlock`, `Grid`, etc.) and automatically generates closing tags or snippet placeholders.
- **Dot-Notation Property Elements:** Supports nested property tags such as `<Grid.RowDefinitions>` and `<Window.Styles>`.
- **Attributes & Events:** Provides suggestions for control properties, events, and attached properties (e.g., `Grid.Row`), complete with inherited members across the inheritance chain.
- **XAML Directives:** Built-in support for standard `x:` attributes (`x:Name`, `x:DataType`, `x:Class`, `x:Key`, `x:CompileBindings`).
- **Documentation Hovers:** View C# summaries, types, and descriptions instantly by hovering over tags, properties, or events.
- **Semantic Token Highlighting:** Differentiates properties and events inside XAML attributes.

---

## Architecture

The project is split into two main utilities:

1. **`axaml-meta`** — A metadata extractor that scans `.dll` and `.xml` documentation files (from Avalonia NuGet packages) and serializes them into a compact JSON index (`meta.json`).
2. **`axaml-ls`** — The LSP server itself, which indexes the layout in memory and serves real-time completion, hover, and token requests to Zed.

---

## Installation & Setup

### 1. Prerequisites

- Rust toolchain (stable)
- Cargo

### 2. Build and Install

Clone the repository and install the LSP binary to your system PATH:

```bash
git clone https://github.com/YaeKitsune13/zed-axaml.git
cd zed-axaml/axaml-ls
cargo build --release
sudo install -Dm755 target/release/axaml-ls /usr/local/bin/axaml-ls

```

### 3. Generate Metadata (`meta.json`)

`axaml-ls` expects a metadata index file to function. By default, it looks at:
`~/.local/share/axaml-ls/meta.json` (or you can override it via the `AXAML_META` environment variable).

Use `axaml-meta` to generate this file from your Avalonia reference assemblies and XML documentation files:

```bash
cd ../axaml-meta
cargo run --release -- /path/to/avalonia/dlls/ folder > ~/.local/share/axaml-ls/meta.json

```

---

## Zed Configuration

To make Zed use `axaml-ls` for `.axaml` files, add the following language server configuration to your Zed settings (`~/.config/zed/settings.json`):

```json
{
  "languages": {
    "XAML": {
      "language_servers": ["axaml-ls"]
    }
  },
  "lsp": {
    "axaml-ls": {}
  }
}
```

---

## License & Third-Party

This project is open-source and available under the MIT License.
See [THIRD-PARTY](THIRD-PARTY) for details regarding third-party code used in this repository.
