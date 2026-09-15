# rust-dev-mcp

An MCP (Model Context Protocol) server that connects AI assistants to [rust-analyzer](https://rust-analyzer.github.io/). It talks to the MCP client over standard input/output, runs rust-analyzer as a subprocess, and uses LSP (Language Server Protocol) to expose Rust IDE capabilities — hover documentation, go-to-definition, find references, completion, diagnostics, rename, and more — as MCP tools.

## Features

The server exposes the following MCP tools:

**Navigation**

| Tool | Description |
| --- | --- |
| `hover` | Get hover documentation for a symbol at a position |
| `definition` | Go to the definition of a symbol |
| `implementation` | Find all implementations of a trait or trait method |
| `type_definition` | Jump to the type definition of a symbol |
| `references` | Find all references to a symbol |
| `workspace_symbols` | Search for symbols by name across the entire workspace |

**Editing assistance**

| Tool | Description |
| --- | --- |
| `completion` | Get code completion suggestions at a position |
| `signature_help` | Get function/method signature and parameter documentation at a call site |
| `inlay_hints` | Get inferred type and parameter-name hints for a range of lines |
| `symbols` | List document symbols (functions, structs, enums, …) |
| `code_actions` | Get code actions (quick-fixes, refactors) for a range |
| `format` | Format a file via rustfmt (returns text edits) |
| `rename` | Rename a symbol across the workspace (returns edits, does not write files) |
| `expand_macro` | Expand a macro invocation and return the generated code |

**Diagnostics**

| Tool | Description |
| --- | --- |
| `diagnostics` | Get compiler diagnostics for a file (triggers a fresh cargo check) |
| `workspace_diagnostics` | Get all diagnostics across the workspace |

**File I/O**

| Tool | Description |
| --- | --- |
| `read_file` | Read the full content of a file from disk |
| `write_file` | Write content to a file (creates missing directories; syncs to rust-analyzer) |
| `apply_edit` | Apply a list of LSP text edits to a file and write the result to disk |

**Workspace**

| Tool | Description |
| --- | --- |
| `set_workspace` | Switch the workspace root that rust-analyzer analyses |

Position arguments `line` and `character` are both 0-based; `character` counts UTF-16 code units, matching the LSP protocol.

## Requirements

- A Rust toolchain
- `rust-analyzer`: available on `PATH`, or installed at `~/.cargo/bin/rust-analyzer`

```bash
rustup component add rust-analyzer
```

## Building

```bash
cargo build --release
```

The resulting binary is `target/release/rust-dev-mcp`.

## Usage

```
rust-dev-mcp [OPTIONS] [--] [WORKSPACE]
```

- `WORKSPACE`: workspace root directory, defaults to the current directory.

Options:

| Option | Description |
| --- | --- |
| `--all-features` | Analyse with every cargo feature enabled |
| `--features <LIST>` | Enable comma- or space-separated features (repeatable) |
| `--no-default-features` | Omit manifest default features |
| `--config <KEY=VALUE>` | Set any rust-analyzer option, e.g. `--config check.command=clippy` |
| `-h`, `--help` | Print help |
| `-V`, `--version` | Print version |

Examples:

```bash
rust-dev-mcp /path/to/project
rust-dev-mcp --features serde,tokio /path/to/project
rust-dev-mcp --config check.command=clippy /path/to/project
```

## MCP integration

The server runs over stdio and speaks MCP protocol version `2024-11-05`. In an MCP client, configure it with the workspace path in `args`:

```json
{
  "mcpServers": {
    "rust-dev-mcp": {
      "command": "rust-dev-mcp",
      "args": ["/path/to/workspace"]
    }
  }
}
```

## Tool examples

`hover` — get documentation for a symbol:

```json
{ "file_path": "src/main.rs", "line": 0, "character": 4 }
```

`workspace_symbols` — find all types named `Config`:

```json
{ "query": "Config" }
```

`inlay_hints` — show inferred types for lines 10–20:

```json
{ "file_path": "src/lib.rs", "start_line": 10, "end_line": 20 }
```

`expand_macro` — expand the macro at a given position:

```json
{ "file_path": "src/main.rs", "line": 5, "character": 4 }
```

`rename` — rename a symbol across the workspace:

```json
{ "file_path": "src/lib.rs", "line": 42, "character": 10, "new_name": "new_identifier" }
```

`rename` returns `applied: false` with a list of `changes`. Apply them by calling `apply_edit` for each affected file:

```json
{
  "file_path": "src/lib.rs",
  "edits": [
    {
      "range": { "start": { "line": 42, "character": 4 }, "end": { "line": 42, "character": 14 } },
      "newText": "new_identifier"
    }
  ]
}
```

`apply_edit` sorts the edits automatically and applies them from the end of the file backwards, so byte offsets stay correct regardless of the order you pass them in.

`write_file` — write (or overwrite) a file:

```json
{ "file_path": "src/lib.rs", "content": "fn main() {}\n" }
```

## How it works

1. The MCP client sends JSON-RPC requests over stdio.
2. The server parses each request. For LSP-backed tools, it ensures the rust-analyzer subprocess is running (restarting it if it has crashed).
3. The server reads the requested file from disk and syncs its content into rust-analyzer.
4. The server sends the corresponding LSP request to rust-analyzer and returns the formatted result.
5. `read_file` and `write_file` operate directly on disk without starting rust-analyzer. When `write_file` is called and rust-analyzer is already running, the new content is synced immediately so subsequent LSP calls see the update.

By default, rust-analyzer starts with cargo build scripts, check-on-save, diagnostics, experimental diagnostics, and proc-macro support enabled. These can be adjusted with `--features`, `--no-default-features`, `--all-features`, and `--config`.

## License

MIT
