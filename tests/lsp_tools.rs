//! End-to-end tests for the LSP-backed tools. These require rust-analyzer.

mod common;

use common::{pos_after, pos_of, retry_tool_until, Client};
use serde_json::json;

const LIB: &str = "src/lib.rs";

fn spawn_simple() -> (Client, String) {
    let client = Client::spawn(&common::fixture("simple"));
    let source = common::fixture_file("simple", LIB);
    (client, source)
}

fn ra_available() -> bool {
    if common::rust_analyzer_available() {
        true
    } else {
        eprintln!("SKIP: rust-analyzer not found on PATH or ~/.cargo/bin");
        false
    }
}

/// Waits until rust-analyzer has indexed the simple fixture, using hover on
/// `add` as a semantic readiness probe.
fn wait_simple_ready(client: &mut Client, source: &str) {
    let (line, character) = pos_of(source, "add(a: i32");
    common::wait_until_hover_ready(client, LIB, line, character);
}

#[test]
fn symbols_and_workspace_symbols_find_fixture_items() {
    if !ra_available() {
        return;
    }
    let (mut client, source) = spawn_simple();
    wait_simple_ready(&mut client, &source);

    let symbols = client.call_tool("symbols", json!({ "file_path": LIB }));
    let names: Vec<&str> = symbols
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    for expected in ["add", "Point", "Describe", "messy"] {
        assert!(names.contains(&expected), "missing {expected} in {names:?}");
    }

    let found = client.call_tool("workspace_symbols", json!({ "query": "Point" }));
    let names: Vec<&str> = found
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert!(names.contains(&"Point"), "workspace_symbols: {found}");
}

#[test]
fn hover_definition_and_references_resolve() {
    if !ra_available() {
        return;
    }
    let (mut client, source) = spawn_simple();
    wait_simple_ready(&mut client, &source);

    // Hover on the definition of `add`.
    let (line, character) = pos_of(&source, "add(a: i32");
    let hover = client.call_tool(
        "hover",
        json!({ "file_path": LIB, "line": line, "character": character }),
    );
    assert!(hover.to_string().contains("Adds two numbers"), "hover: {hover}");

    // Definition from the call site points back at the function.
    let (call_line, call_char) = pos_of(&source, "add(2, 3)");
    let definition = client.call_tool(
        "definition",
        json!({ "file_path": LIB, "line": call_line, "character": call_char }),
    );
    let target = &definition[0];
    assert!(
        target["targetUri"].as_str().unwrap().ends_with(LIB),
        "definition: {definition}"
    );
    let (def_line, _) = pos_of(&source, "add(a: i32");
    assert_eq!(target["targetSelectionRange"]["start"]["line"], def_line);

    // References include the struct definition and its impl blocks.
    let (point_line, point_char) = pos_of(&source, "Point {");
    let references = client.call_tool(
        "references",
        json!({ "file_path": LIB, "line": point_line, "character": point_char }),
    );
    let count = references.as_array().unwrap().len();
    assert!(count >= 3, "expected >= 3 Point references, got {count}: {references}");
}

#[test]
fn type_definition_and_implementation_resolve() {
    if !ra_available() {
        return;
    }
    let (mut client, source) = spawn_simple();
    wait_simple_ready(&mut client, &source);

    let (line, character) = pos_of(&source, "Point { x: 1");
    let type_def = client.call_tool(
        "type_definition",
        json!({ "file_path": LIB, "line": line, "character": character }),
    );
    let target = &type_def[0];
    assert!(
        target["targetUri"].as_str().unwrap().ends_with(LIB),
        "type_definition: {type_def}"
    );

    let (trait_line, trait_char) = pos_of(&source, "Describe");
    let impls = client.call_tool(
        "implementation",
        json!({ "file_path": LIB, "line": trait_line, "character": trait_char }),
    );
    let arr = impls.as_array().unwrap();
    assert!(!arr.is_empty(), "no implementations found: {impls}");
    assert!(arr[0]["targetUri"].as_str().unwrap().ends_with(LIB));
}

#[test]
fn completion_signature_help_and_inlay_hints_work() {
    if !ra_available() {
        return;
    }
    let (mut client, source) = spawn_simple();
    wait_simple_ready(&mut client, &source);

    // Completion right after `Point::`. Completion can return null or an empty
    // list while rust-analyzer's completion index is still warming up, so poll.
    let (line, character) = pos_after(&source, "Point::");
    let completion = retry_tool_until(
        &mut client,
        "completion",
        json!({ "file_path": LIB, "line": line, "character": character }),
        |v| v["items"].as_array().is_some_and(|a| !a.is_empty()),
    );
    let labels: Vec<&str> = completion["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i["label"].as_str())
        .collect();
    assert!(labels.contains(&"origin"), "completion missing origin: {labels:?}");

    // Signature help inside the `add(2, 3)` call site. Use the unique call
    // marker here: the bare string "add(" also matches the definition line.
    let (sig_line, sig_char) = pos_of(&source, "add(2, 3)");
    let sig_char = sig_char + "add(".len() as u32;
    let signature = retry_tool_until(
        &mut client,
        "signature_help",
        json!({ "file_path": LIB, "line": sig_line, "character": sig_char }),
        |v| !v.is_null(),
    );
    assert!(
        signature.to_string().contains("fn add(a: i32, b: i32) -> i32"),
        "signature: {signature}"
    );

    // Inlay hints for `let value = 42;`.
    let (hint_line, _) = pos_of(&source, "let value = 42;");
    let hints = retry_tool_until(
        &mut client,
        "inlay_hints",
        json!({ "file_path": LIB, "start_line": hint_line, "end_line": hint_line }),
        |v| v.as_array().is_some_and(|a| !a.is_empty()),
    );
    assert!(hints.to_string().contains("i32"), "inlay hints: {hints}");
}

#[test]
fn expand_macro_expands_local_macro() {
    if !ra_available() {
        return;
    }
    let (mut client, source) = spawn_simple();
    wait_simple_ready(&mut client, &source);

    let (line, character) = pos_of(&source, "double!(21)");
    let expansion = client.call_tool(
        "expand_macro",
        json!({ "file_path": LIB, "line": line, "character": character }),
    );
    assert!(
        expansion.to_string().contains("21 * 2"),
        "macro expansion: {expansion}"
    );
}

#[test]
fn format_returns_edits_and_code_actions_offer_refactors() {
    if !ra_available() {
        return;
    }
    let (mut client, source) = spawn_simple();
    wait_simple_ready(&mut client, &source);

    let edits = client.call_tool("format", json!({ "file_path": LIB }));
    let arr = edits.as_array().unwrap();
    assert!(!arr.is_empty(), "expected format edits for messy(): {edits}");
    let combined: String = arr.iter().filter_map(|e| e["newText"].as_str()).collect();
    assert!(combined.contains("1 + 1"), "formatted edits: {combined}");

    // Code actions on `value` should offer an explicit type insertion.
    let (line, character) = pos_of(&source, "let value = 42;");
    let start = character + "let ".len() as u32;
    let end = start + "value".len() as u32;
    let actions = client.call_tool(
        "code_actions",
        json!({
            "file_path": LIB,
            "line": line,
            "character": start,
            "end_line": line,
            "end_character": end
        }),
    );
    let titles: Vec<&str> = actions
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|a| a["title"].as_str())
        .collect();
    assert!(
        titles.iter().any(|t| t.contains("Insert explicit type")),
        "actions: {titles:?}"
    );
}

#[test]
fn rename_returns_edits_without_modifying_files() {
    if !ra_available() {
        return;
    }
    let (mut client, source) = spawn_simple();
    wait_simple_ready(&mut client, &source);
    let original = source.clone();

    let (line, character) = pos_of(&source, "add(a: i32");
    let rename = client.call_tool(
        "rename",
        json!({
            "file_path": LIB,
            "line": line,
            "character": character,
            "new_name": "sum"
        }),
    );

    assert_eq!(rename["applied"], false);
    assert_eq!(rename["old_name"], "add");
    assert_eq!(rename["new_name"], "sum");
    assert_eq!(rename["summary"]["edits"], 2, "rename: {rename}");

    let edits = rename["changes"][0]["edits"].as_array().unwrap();
    let old_texts: Vec<&str> = edits.iter().filter_map(|e| e["old_text"].as_str()).collect();
    let new_texts: Vec<&str> = edits.iter().filter_map(|e| e["new_text"].as_str()).collect();
    assert!(old_texts.contains(&"add"), "old_texts: {old_texts:?}");
    assert!(new_texts.iter().all(|t| *t == "sum"), "new_texts: {new_texts:?}");

    // The on-disk fixture must be untouched.
    let after = common::fixture_file("simple", LIB);
    assert_eq!(after, original, "rename modified the fixture file");
}

#[test]
fn diagnostics_report_real_compiler_errors() {
    if !ra_available() {
        return;
    }
    let mut client = Client::spawn(&common::fixture("broken"));

    // `diagnostics` waits internally for load + cargo check to finish.
    let result = client.call_tool("diagnostics", json!({ "file_path": "src/main.rs" }));
    assert!(
        result["summary"]["errors"].as_u64().unwrap_or(0) >= 2,
        "diagnostics: {result}"
    );
    let text = result.to_string();
    assert!(text.contains("E0308"), "missing E0308: {result}");
    assert!(text.contains("E0425"), "missing E0425: {result}");
}

#[test]
fn workspace_diagnostics_and_set_workspace_reflect_current_workspace() {
    if !ra_available() {
        return;
    }
    let mut client = Client::spawn(&common::fixture("simple"));
    let source = common::fixture_file("simple", LIB);
    wait_simple_ready(&mut client, &source);

    let clean = client.call_tool("workspace_diagnostics", json!({}));
    assert_eq!(clean["summary"]["total_errors"], 0, "clean: {clean}");

    let broken_path = common::fixture("broken");
    let result = client.call_tool_text(
        "set_workspace",
        json!({ "workspace_path": broken_path.to_str().unwrap() }),
    );
    assert!(result.contains("Workspace set to"), "{result}");

    // Force the new workspace to finish loading + checking, then assert the
    // workspace-level diagnostics reflect it.
    let fresh = client.call_tool("diagnostics", json!({ "file_path": "src/main.rs" }));
    assert!(
        fresh["summary"]["errors"].as_u64().unwrap_or(0) >= 2,
        "fresh diagnostics after switch: {fresh}"
    );

    let broken = client.call_tool("workspace_diagnostics", json!({}));
    assert!(
        broken["summary"]["total_errors"].as_u64().unwrap_or(0) >= 2,
        "broken: {broken}"
    );
}
