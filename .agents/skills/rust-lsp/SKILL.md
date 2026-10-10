---
name: rust-lsp
description: Use Rust code intelligence (hover, go-to-definition, references, rename, diagnostics, symbols) via the rust-lsp-mcp server. Use when you need to understand, navigate, or refactor a Rust codebase.
---

# rust-lsp

Rust IDE capabilities through `rust-analyzer`, exposed as MCP tools.

## Position conventions

- `line` and `character` are **0-based**; `character` counts **UTF-16 code units** (LSP convention).
- `file_path` may be relative to the workspace root, absolute, or a `file://` URI.

## Tool reference

| Tool | Args | Returns |
| --- | --- | --- |
| `hover` | file_path, line, character | doc + type |
| `definition` / `implementation` / `type_definition` | file_path, line, character | location |
| `references` | file_path, line, character | list of locations |
| `workspace_symbols` | query | matching symbols |
| `symbols` | file_path | symbols in file |
| `completion` / `signature_help` | file_path, line, character | suggestions / signature |
| `inlay_hints` | file_path, start_line, end_line | inferred types |
| `code_actions` | file_path, line, character, end_line, end_character | quick-fixes |
| `format` | file_path | text edits |
| `rename` | file_path, line, character, new_name | text edits (no write) |
| `expand_macro` | file_path, line, character | expanded code |
| `diagnostics` | file_path | compiler diagnostics |
| `workspace_diagnostics` | — | all diagnostics |
| `set_workspace` | workspace_path | — |
| `read_file` / `write_file` | file_path [, content] | file content / status |
| `apply_edit` | file_path, edits | writes edits to disk |

## Notes

- `rename`, `format`, and `code_actions` return edits and **do not write files**; persist them with `apply_edit`.
- `diagnostics` runs a fresh `cargo check` and is slow on a cold cache.
- `set_workspace` takes `workspace_path` (not `workspace`).
