use crate::protocol::ToolDefinition;
use serde_json::json;

pub fn list() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "hover".into(),
            description: "Get hover documentation for a symbol at a position in a Rust file".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "Rust file path (relative, absolute, or file:// URI)" },
                    "line":      { "type": "integer", "description": "Line number (0-based)" },
                    "character": { "type": "integer", "description": "Character offset (0-based, UTF-16 code units)" }
                },
                "required": ["file_path", "line", "character"]
            }),
        },
        ToolDefinition {
            name: "definition".into(),
            description: "Go to the definition of a symbol".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string" },
                    "line":      { "type": "integer" },
                    "character": { "type": "integer" }
                },
                "required": ["file_path", "line", "character"]
            }),
        },
        ToolDefinition {
            name: "references".into(),
            description: "Find all references to a symbol".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string" },
                    "line":      { "type": "integer" },
                    "character": { "type": "integer" }
                },
                "required": ["file_path", "line", "character"]
            }),
        },
        ToolDefinition {
            name: "completion".into(),
            description: "Get code completion suggestions at a position".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string" },
                    "line":      { "type": "integer" },
                    "character": { "type": "integer" }
                },
                "required": ["file_path", "line", "character"]
            }),
        },
        ToolDefinition {
            name: "symbols".into(),
            description: "List document symbols (functions, structs, enums, …)".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string" }
                },
                "required": ["file_path"]
            }),
        },
        ToolDefinition {
            name: "format".into(),
            description: "Format a Rust file via rustfmt (returns text edits)".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string" }
                },
                "required": ["file_path"]
            }),
        },
        ToolDefinition {
            name: "code_actions".into(),
            description: "Get available code actions (quick-fixes, refactors) for a range".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path":     { "type": "string" },
                    "line":          { "type": "integer", "description": "Start line (0-based)" },
                    "character":     { "type": "integer", "description": "Start character (0-based)" },
                    "end_line":      { "type": "integer", "description": "End line (0-based)" },
                    "end_character": { "type": "integer", "description": "End character (0-based)" }
                },
                "required": ["file_path", "line", "character", "end_line", "end_character"]
            }),
        },
        ToolDefinition {
            name: "rename".into(),
            description: "\
                Rename a symbol across the workspace and return the edits. \
                No files are modified; apply the returned edits yourself. \
                Fails if rust-analyzer is still loading or nothing is renameable at the position."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string" },
                    "line":      { "type": "integer" },
                    "character": { "type": "integer" },
                    "new_name":  { "type": "string", "description": "New identifier (include leading ' for lifetimes)" }
                },
                "required": ["file_path", "line", "character", "new_name"]
            }),
        },
        ToolDefinition {
            name: "diagnostics".into(),
            description: "Get compiler diagnostics for a file (triggers a fresh cargo check)".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string" }
                },
                "required": ["file_path"]
            }),
        },
        ToolDefinition {
            name: "workspace_diagnostics".into(),
            description: "Get all diagnostics across the workspace".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "set_workspace".into(),
            description: "Switch the workspace root that rust-analyzer analyses".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "workspace_path": { "type": "string", "description": "New workspace root (path or file:// URI)" }
                },
                "required": ["workspace_path"]
            }),
        },
        ToolDefinition {
            name: "implementation".into(),
            description: "Find all implementations of a trait method or trait itself at the given position".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string" },
                    "line":      { "type": "integer", "description": "Line number (0-based)" },
                    "character": { "type": "integer", "description": "Character offset (0-based, UTF-16 code units)" }
                },
                "required": ["file_path", "line", "character"]
            }),
        },
        ToolDefinition {
            name: "type_definition".into(),
            description: "Jump to the type definition of the symbol at the given position".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string" },
                    "line":      { "type": "integer" },
                    "character": { "type": "integer" }
                },
                "required": ["file_path", "line", "character"]
            }),
        },
        ToolDefinition {
            name: "workspace_symbols".into(),
            description: "Search for symbols (functions, structs, enums, …) by name across the entire workspace".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Symbol name or prefix to search for" }
                },
                "required": ["query"]
            }),
        },
        ToolDefinition {
            name: "inlay_hints".into(),
            description: "Get inlay hints (inferred types, parameter names) for a range of lines in a file".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path":  { "type": "string" },
                    "start_line": { "type": "integer", "description": "First line (0-based, inclusive)" },
                    "end_line":   { "type": "integer", "description": "Last line (0-based, inclusive)" }
                },
                "required": ["file_path", "start_line", "end_line"]
            }),
        },
        ToolDefinition {
            name: "signature_help".into(),
            description: "Get function/method signature help (parameter types and documentation) at a call site".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string" },
                    "line":      { "type": "integer" },
                    "character": { "type": "integer" }
                },
                "required": ["file_path", "line", "character"]
            }),
        },
        ToolDefinition {
            name: "expand_macro".into(),
            description: "Expand a macro invocation and return the generated code (rust-analyzer extension)".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string" },
                    "line":      { "type": "integer" },
                    "character": { "type": "integer" }
                },
                "required": ["file_path", "line", "character"]
            }),
        },
        ToolDefinition {
            name: "write_file".into(),
            description: "\
                Write text content to a file on disk. \
                Creates the file (and any missing parent directories) if it does not exist, \
                or overwrites it if it does. \
                Use this to apply edits returned by format, code_actions, or rename."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "Destination file path (relative to workspace root or absolute)" },
                    "content":   { "type": "string", "description": "Full file content to write" }
                },
                "required": ["file_path", "content"]
            }),
        },
        ToolDefinition {
            name: "read_file".into(),
            description: "Read the full text content of a file from disk".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string" }
                },
                "required": ["file_path"]
            }),
        },
        ToolDefinition {
            name: "apply_edit".into(),
            description: "\
                Apply a list of LSP text edits to a file and write the result to disk. \
                Edits are the objects returned by format, code_actions, or rename \
                (each has a `range` with `start`/`end` line+character and a `newText`). \
                Multiple edits are applied atomically in a single write; \
                no need to sort them beforehand."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "File to modify (relative to workspace root or absolute)"
                    },
                    "edits": {
                        "type": "array",
                        "description": "LSP TextEdit objects to apply",
                        "items": {
                            "type": "object",
                            "properties": {
                                "range": {
                                    "type": "object",
                                    "properties": {
                                        "start": {
                                            "type": "object",
                                            "properties": {
                                                "line":      { "type": "integer" },
                                                "character": { "type": "integer" }
                                            },
                                            "required": ["line", "character"]
                                        },
                                        "end": {
                                            "type": "object",
                                            "properties": {
                                                "line":      { "type": "integer" },
                                                "character": { "type": "integer" }
                                            },
                                            "required": ["line", "character"]
                                        }
                                    },
                                    "required": ["start", "end"]
                                },
                                "newText": { "type": "string" }
                            },
                            "required": ["range", "newText"]
                        }
                    }
                },
                "required": ["file_path", "edits"]
            }),
        },
    ]
}
