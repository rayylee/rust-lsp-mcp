use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::{
    cmp::Reverse,
    path::{Path, PathBuf},
    time::Duration,
};

use crate::{
    constants::WORKSPACE_LOAD_TIMEOUT_SECS,
    position,
    protocol::{ContentItem, ToolResult},
    uri,
};

use super::server::McpServer;

pub async fn dispatch(server: &mut McpServer, tool: &str, args: Value) -> Result<ToolResult> {
    // File I/O tools don't need rust-analyzer; skip the startup cost.
    match tool {
        "write_file" => return write_file(server, args).await,
        "read_file"  => return read_file(server, args).await,
        "apply_edit" => return apply_edit(server, args).await,
        _ => {}
    }

    server.ensure_client().await?;

    match tool {
        "hover"                => hover(server, args).await,
        "definition"           => definition(server, args).await,
        "references"           => references(server, args).await,
        "completion"           => completion(server, args).await,
        "symbols"              => symbols(server, args).await,
        "format"               => run_format(server, args).await,
        "code_actions"         => code_actions(server, args).await,
        "rename"               => rename(server, args).await,
        "diagnostics"          => diagnostics(server, args).await,
        "workspace_diagnostics" => workspace_diagnostics(server, args).await,
        "set_workspace"         => set_workspace(server, args).await,
        "implementation"        => implementation(server, args).await,
        "type_definition"       => type_definition(server, args).await,
        "workspace_symbols"     => workspace_symbols(server, args).await,
        "inlay_hints"           => inlay_hints(server, args).await,
        "signature_help"        => signature_help(server, args).await,
        "expand_macro"          => expand_macro(server, args).await,
        "write_file"            => write_file(server, args).await,
        "read_file"             => read_file(server, args).await,
        _ => Err(anyhow!("Unknown tool: {}", tool)),
    }
}

// ---------------------------------------------------------------------------
// Parameter extraction helpers
// ---------------------------------------------------------------------------

fn extract_file_path(args: &Value) -> Result<String> {
    args["file_path"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("Missing 'file_path'"))
}

fn extract_pos(args: &Value) -> Result<(u32, u32)> {
    let line = args["line"]
        .as_u64()
        .ok_or_else(|| anyhow!("Missing 'line'"))? as u32;
    let character = args["character"]
        .as_u64()
        .ok_or_else(|| anyhow!("Missing 'character'"))? as u32;
    Ok((line, character))
}

fn extract_range(args: &Value) -> Result<(u32, u32, u32, u32)> {
    let (line, character) = extract_pos(args)?;
    let end_line = args["end_line"]
        .as_u64()
        .ok_or_else(|| anyhow!("Missing 'end_line'"))? as u32;
    let end_character = args["end_character"]
        .as_u64()
        .ok_or_else(|| anyhow!("Missing 'end_character'"))? as u32;
    Ok((line, character, end_line, end_character))
}

fn into_tool_result(v: Value) -> Result<ToolResult> {
    Ok(ToolResult {
        content: vec![ContentItem::text(serde_json::to_string_pretty(&v)?)],
    })
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

async fn hover(server: &mut McpServer, args: Value) -> Result<ToolResult> {
    let fp = extract_file_path(&args)?;
    let (line, character) = extract_pos(&args)?;
    let uri = server.load_file(&fp).await?;
    let result = server.lsp().hover(&uri, line, character).await?;
    into_tool_result(result)
}

async fn definition(server: &mut McpServer, args: Value) -> Result<ToolResult> {
    let fp = extract_file_path(&args)?;
    let (line, character) = extract_pos(&args)?;
    let uri = server.load_file(&fp).await?;
    let result = server.lsp().definition(&uri, line, character).await?;
    into_tool_result(result)
}

async fn references(server: &mut McpServer, args: Value) -> Result<ToolResult> {
    let fp = extract_file_path(&args)?;
    let (line, character) = extract_pos(&args)?;
    let uri = server.load_file(&fp).await?;
    let result = server.lsp().references(&uri, line, character).await?;
    into_tool_result(result)
}

async fn completion(server: &mut McpServer, args: Value) -> Result<ToolResult> {
    let fp = extract_file_path(&args)?;
    let (line, character) = extract_pos(&args)?;
    let uri = server.load_file(&fp).await?;
    let result = server.lsp().completion(&uri, line, character).await?;
    into_tool_result(result)
}

async fn symbols(server: &mut McpServer, args: Value) -> Result<ToolResult> {
    let fp = extract_file_path(&args)?;
    let uri = server.load_file(&fp).await?;
    let result = server.lsp().document_symbols(&uri).await?;
    into_tool_result(result)
}

async fn run_format(server: &mut McpServer, args: Value) -> Result<ToolResult> {
    let fp = extract_file_path(&args)?;
    let uri = server.load_file(&fp).await?;
    let result = server.lsp().formatting(&uri).await?;
    into_tool_result(result)
}

async fn code_actions(server: &mut McpServer, args: Value) -> Result<ToolResult> {
    let fp = extract_file_path(&args)?;
    let (line, character, end_line, end_character) = extract_range(&args)?;
    let uri = server.load_file(&fp).await?;
    let result = server
        .lsp()
        .code_actions(&uri, line, character, end_line, end_character)
        .await?;
    into_tool_result(result)
}

async fn rename(server: &mut McpServer, args: Value) -> Result<ToolResult> {
    let fp = extract_file_path(&args)?;
    let (line, character) = extract_pos(&args)?;
    let new_name = args["new_name"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing 'new_name'"))?
        .to_string();

    let uri = server.load_file(&fp).await?;
    server.refresh_open_docs().await?;

    {
        if !server
            .lsp()
            .wait_until_loaded(Duration::from_secs(WORKSPACE_LOAD_TIMEOUT_SECS))
            .await
        {
            return Err(anyhow!(
                "rust-analyzer is still loading the workspace after {}s; \
                 a rename now could miss references — try again once it has settled.",
                WORKSPACE_LOAD_TIMEOUT_SECS
            ));
        }
    }

    let renaming = server.lsp().prepare_rename(&uri, line, character).await?;
    let edit = server.lsp().rename(&uri, line, character, &new_name).await?;

    if edit.is_null() {
        return Err(anyhow!(
            "Nothing to rename at {}:{}:{}",
            fp,
            line,
            character
        ));
    }

    let old_name = extract_old_name(server, &fp, &renaming).await;
    let result = describe_rename(&edit, old_name, &new_name).await;
    into_tool_result(result)
}

async fn diagnostics(server: &mut McpServer, args: Value) -> Result<ToolResult> {
    let fp = extract_file_path(&args)?;
    let uri = server.load_file(&fp).await?;
    let fresh = server.lsp().fresh_diagnostics(&uri).await?;

    let mut out = format_diagnostics(&fp, &fresh.items);
    if !fresh.complete {
        out["note"] = json!(
            "rust-analyzer had not finished loading or checking when this was reported; \
             results may be incomplete. Ask again for the full picture."
        );
    }
    into_tool_result(out)
}

async fn workspace_diagnostics(server: &mut McpServer, _args: Value) -> Result<ToolResult> {
    let result = server.lsp().workspace_diagnostics().await?;
    let formatted = format_workspace_diagnostics(&server.workspace_root, &result);
    into_tool_result(formatted)
}

async fn set_workspace(server: &mut McpServer, args: Value) -> Result<ToolResult> {
    let ws = args["workspace_path"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing 'workspace_path'"))?;

    if let Some(client) = &mut server.client {
        client.shutdown().await?;
    }
    server.client = None;

    let root = uri::uri_to_path(ws).unwrap_or_else(|| PathBuf::from(ws));
    server.workspace_root = uri::absolute(&root);
    server.ensure_client().await?;

    Ok(ToolResult {
        content: vec![ContentItem::text(format!(
            "Workspace set to: {}",
            server.workspace_root.display()
        ))],
    })
}

async fn implementation(server: &mut McpServer, args: Value) -> Result<ToolResult> {
    let fp = extract_file_path(&args)?;
    let (line, character) = extract_pos(&args)?;
    let uri = server.load_file(&fp).await?;
    let result = server.lsp().implementation(&uri, line, character).await?;
    into_tool_result(result)
}

async fn type_definition(server: &mut McpServer, args: Value) -> Result<ToolResult> {
    let fp = extract_file_path(&args)?;
    let (line, character) = extract_pos(&args)?;
    let uri = server.load_file(&fp).await?;
    let result = server.lsp().type_definition(&uri, line, character).await?;
    into_tool_result(result)
}

async fn workspace_symbols(server: &mut McpServer, args: Value) -> Result<ToolResult> {
    let query = args["query"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing 'query'"))?
        .to_string();
    let result = server.lsp().workspace_symbols(&query).await?;
    into_tool_result(result)
}

async fn inlay_hints(server: &mut McpServer, args: Value) -> Result<ToolResult> {
    let fp = extract_file_path(&args)?;
    let start_line = args["start_line"]
        .as_u64()
        .ok_or_else(|| anyhow!("Missing 'start_line'"))? as u32;
    let end_line = args["end_line"]
        .as_u64()
        .ok_or_else(|| anyhow!("Missing 'end_line'"))? as u32;
    let uri = server.load_file(&fp).await?;
    let result = server.lsp().inlay_hints(&uri, start_line, end_line).await?;
    into_tool_result(result)
}

async fn signature_help(server: &mut McpServer, args: Value) -> Result<ToolResult> {
    let fp = extract_file_path(&args)?;
    let (line, character) = extract_pos(&args)?;
    let uri = server.load_file(&fp).await?;
    let result = server.lsp().signature_help(&uri, line, character).await?;
    into_tool_result(result)
}

async fn expand_macro(server: &mut McpServer, args: Value) -> Result<ToolResult> {
    let fp = extract_file_path(&args)?;
    let (line, character) = extract_pos(&args)?;
    let uri = server.load_file(&fp).await?;
    let result = server.lsp().expand_macro(&uri, line, character).await?;
    into_tool_result(result)
}

async fn apply_edit(server: &mut McpServer, args: Value) -> Result<ToolResult> {
    let fp = extract_file_path(&args)?;
    let edits = args["edits"]
        .as_array()
        .ok_or_else(|| anyhow!("Missing 'edits'"))?
        .clone();

    let path = server.resolve_path(&fp);
    let original = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| anyhow!("Cannot read {}: {}", path.display(), e))?;

    // Sort edits descending by start position so applying from the end of the
    // file leaves earlier offsets unaffected.
    let mut sorted = edits.clone();
    sorted.sort_by(|a, b| {
        let pos = |v: &Value, end: &str| {
            let line = v.pointer(&format!("/range/{end}/line")).and_then(|v| v.as_u64()).unwrap_or(0);
            let ch   = v.pointer(&format!("/range/{end}/character")).and_then(|v| v.as_u64()).unwrap_or(0);
            (line, ch)
        };
        pos(b, "start").cmp(&pos(a, "start"))
    });

    let mut result = original.clone();
    for edit in &sorted {
        let range = edit.get("range").ok_or_else(|| anyhow!("Edit missing 'range'"))?;
        let new_text = edit["newText"].as_str().unwrap_or("");

        let get = |end: &str, field: &str| -> Result<u32> {
            range.pointer(&format!("/{end}/{field}"))
                .and_then(|v| v.as_u64())
                .map(|v| v as u32)
                .ok_or_else(|| anyhow!("Edit range missing {end}.{field}"))
        };

        let start = position::byte_offset(&result, get("start", "line")?, get("start", "character")?);
        let end   = position::byte_offset(&result, get("end",   "line")?, get("end",   "character")?);

        if start > end || end > result.len() {
            return Err(anyhow!(
                "Edit range {}:{} – {}:{} is out of bounds (file is {} bytes)",
                get("start", "line")?, get("start", "character")?,
                get("end",   "line")?, get("end",   "character")?,
                result.len()
            ));
        }

        result.replace_range(start..end, new_text);
    }

    tokio::fs::write(&path, result.as_bytes())
        .await
        .map_err(|e| anyhow!("Cannot write {}: {}", path.display(), e))?;

    // Sync to rust-analyzer if it is already running.
    if server.client.is_some() {
        let uri = uri::path_to_uri(&path)?;
        server.lsp().open_document(&uri, &result).await?;
    }

    Ok(ToolResult {
        content: vec![ContentItem::text(format!(
            "Applied {} edit(s) to {} ({} bytes)",
            sorted.len(),
            path.display(),
            result.len(),
        ))],
    })
}

async fn write_file(server: &mut McpServer, args: Value) -> Result<ToolResult> {
    let fp = extract_file_path(&args)?;
    let content = args["content"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing 'content'"))?
        .to_string();

    let path = server.resolve_path(&fp);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            anyhow!("Cannot create directories for {}: {}", path.display(), e)
        })?;
    }
    tokio::fs::write(&path, content.as_bytes())
        .await
        .map_err(|e| anyhow!("Cannot write {}: {}", path.display(), e))?;

    // Sync the new content into rust-analyzer if it is already running.
    if server.client.is_some() {
        let uri = uri::path_to_uri(&path)?;
        server.lsp().open_document(&uri, &content).await?;
    }

    Ok(ToolResult {
        content: vec![ContentItem::text(format!(
            "Written {} bytes to {}",
            content.len(),
            path.display()
        ))],
    })
}

async fn read_file(server: &mut McpServer, args: Value) -> Result<ToolResult> {
    let fp = extract_file_path(&args)?;
    let path = server.resolve_path(&fp);
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| anyhow!("Cannot read {}: {}", path.display(), e))?;
    Ok(ToolResult {
        content: vec![ContentItem::text(content)],
    })
}

// ---------------------------------------------------------------------------
// Rename description helpers
// ---------------------------------------------------------------------------

async fn extract_old_name(server: &McpServer, file_path: &str, renaming: &Value) -> Option<String> {
    let range = renaming.get("range").unwrap_or(renaming);
    let content = tokio::fs::read_to_string(server.resolve_path(file_path))
        .await
        .ok()?;
    Some(text_of(&content, range)?.to_string())
}

async fn describe_rename(edit: &Value, old_name: Option<String>, new_name: &str) -> Value {
    let mut text_changes: Vec<Value> = Vec::new();
    let mut file_operations: Vec<Value> = Vec::new();
    let mut total_edits = 0usize;

    for change in parse_workspace_edit(edit) {
        match change {
            Change::Text { uri, edits } => {
                let path = uri::uri_to_path(&uri);
                let content = match &path {
                    Some(p) => tokio::fs::read_to_string(p).await.ok(),
                    None => None,
                };

                let mut described: Vec<Value> = edits
                    .iter()
                    .map(|e| describe_edit(e, content.as_deref()))
                    .collect();
                // Descending order so edits can be applied one by one without offset arithmetic.
                described.sort_by_key(|e| {
                    Reverse((
                        e["line"].as_u64().unwrap_or(0),
                        e["character"].as_u64().unwrap_or(0),
                    ))
                });

                total_edits += described.len();
                text_changes.push(json!({
                    "file": display_path(&path, &uri),
                    "edits": described,
                }));
            }
            Change::Resource(op) => file_operations.push(op),
        }
    }

    json!({
        "applied": false,
        "old_name": old_name,
        "new_name": new_name,
        "position_encoding": "utf-16",
        "summary": {
            "files_changed": text_changes.len(),
            "edits": total_edits,
            "file_operations": file_operations.len(),
        },
        "changes": text_changes,
        "file_operations": file_operations,
        "workspace_edit": edit,
    })
}

enum Change {
    Text { uri: String, edits: Vec<Value> },
    Resource(Value),
}

fn parse_workspace_edit(edit: &Value) -> Vec<Change> {
    if let Some(doc_changes) = edit.get("documentChanges").and_then(|v| v.as_array()) {
        return doc_changes
            .iter()
            .map(|c| match c.get("edits").and_then(|e| e.as_array()) {
                Some(edits) => Change::Text {
                    uri: c["textDocument"]["uri"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    edits: edits.clone(),
                },
                None => Change::Resource(c.clone()),
            })
            .collect();
    }

    edit.get("changes")
        .and_then(|v| v.as_object())
        .map(|map| {
            map.iter()
                .map(|(uri, edits)| Change::Text {
                    uri: uri.clone(),
                    edits: edits.as_array().cloned().unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn describe_edit(edit: &Value, content: Option<&str>) -> Value {
    let range = edit.get("range").cloned().unwrap_or(json!(null));
    let mut out = json!({
        "line":          range["start"]["line"],
        "character":     range["start"]["character"],
        "end_line":      range["end"]["line"],
        "end_character": range["end"]["character"],
        "new_text":      edit.get("newText").cloned().unwrap_or(json!("")),
    });

    if let Some(content) = content {
        if let Some((start, end)) = byte_range(content, &range) {
            out["byte_range"] = json!([start, end]);
            out["old_text"] = json!(&content[start..end]);
        }
    }
    out
}

fn text_of<'a>(content: &'a str, range: &Value) -> Option<&'a str> {
    let (start, end) = byte_range(content, range)?;
    Some(&content[start..end])
}

fn byte_range(content: &str, range: &Value) -> Option<(usize, usize)> {
    let get = |end: &str, field: &str| -> Option<u32> {
        range.get(end)?.get(field)?.as_u64().map(|v| v as u32)
    };
    let start = position::byte_offset(content, get("start", "line")?, get("start", "character")?);
    let end = position::byte_offset(content, get("end", "line")?, get("end", "character")?);
    (start <= end).then_some((start, end))
}

fn display_path(path: &Option<PathBuf>, uri: &str) -> String {
    path.as_ref()
        .map_or_else(|| uri.to_string(), |p| p.display().to_string())
}

// ---------------------------------------------------------------------------
// Diagnostics formatting
// ---------------------------------------------------------------------------

fn format_diagnostics(file: &str, items: &Value) -> Value {
    let diags = items.as_array().map(Vec::as_slice).unwrap_or(&[]);
    let mut errors = 0u32;
    let mut warnings = 0u32;
    let mut information = 0u32;
    let mut hints = 0u32;

    let formatted: Vec<Value> = diags
        .iter()
        .map(|d| {
            let sev = d.get("severity").and_then(|s| s.as_u64());
            match sev {
                Some(1) => errors += 1,
                Some(2) => warnings += 1,
                Some(3) => information += 1,
                Some(4) => hints += 1,
                _ => {}
            }
            json!({
                "severity": match sev {
                    Some(1) => "error",
                    Some(2) => "warning",
                    Some(3) => "information",
                    Some(4) => "hint",
                    _ => "unknown",
                },
                "range": d.get("range").cloned().unwrap_or(json!(null)),
                "message": d["message"].as_str().unwrap_or(""),
                "code": d.get("code").cloned().unwrap_or(json!(null)),
                "source": d["source"].as_str().unwrap_or("rust-analyzer"),
                "relatedInformation": d.get("relatedInformation").cloned().unwrap_or(json!(null)),
            })
        })
        .collect();

    json!({
        "file": file,
        "diagnostics": formatted,
        "summary": {
            "errors": errors,
            "warnings": warnings,
            "information": information,
            "hints": hints,
        }
    })
}

fn format_workspace_diagnostics(workspace_root: &Path, result: &Value) -> Value {
    // Pull model: `{ "kind": "full", "items": [...] }`
    if let Some(items) = result.get("items").and_then(|v| v.as_array()) {
        let errors = count_severity(items, 1);
        let warnings = count_severity(items, 2);
        return json!({
            "workspace": workspace_root.display().to_string(),
            "kind": "full",
            "items": items,
            "summary": {
                "total": items.len(),
                "errors": errors,
                "warnings": warnings,
            }
        });
    }

    // Push model fallback: `{ "file_uri": [diags], ... }`
    let Some(map) = result.as_object() else {
        return json!({
            "workspace": workspace_root.display().to_string(),
            "raw": result,
        });
    };

    let mut files = serde_json::Map::new();
    let mut total_errors = 0u32;
    let mut total_warnings = 0u32;

    for (uri, diags) in map {
        let arr = match diags.as_array() {
            Some(a) if !a.is_empty() => a,
            _ => continue,
        };
        let errors = count_severity(arr, 1);
        let warnings = count_severity(arr, 2);
        total_errors += errors;
        total_warnings += warnings;
        files.insert(
            uri.clone(),
            json!({
                "diagnostics": diags,
                "summary": { "errors": errors, "warnings": warnings }
            }),
        );
    }

    json!({
        "workspace": workspace_root.display().to_string(),
        "files": files,
        "summary": {
            "total_files": files.len(),
            "total_errors": total_errors,
            "total_warnings": total_warnings,
        }
    })
}

fn count_severity(diags: &[Value], severity: u64) -> u32 {
    diags
        .iter()
        .filter(|d| d.get("severity").and_then(|s| s.as_u64()) == Some(severity))
        .count() as u32
}
