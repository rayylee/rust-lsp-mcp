//! Protocol-level tests that need no rust-analyzer and run quickly.

mod common;

use common::Client;
use serde_json::json;
use std::collections::BTreeSet;

#[test]
fn initialize_reports_protocol_version_and_server_info() {
    let mut client = Client::spawn(&common::fixture("simple"));
    let response = client.request("initialize", Some(json!({})));
    assert!(response.get("error").is_none(), "initialize failed: {response}");
    assert_eq!(response["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(response["result"]["serverInfo"]["name"], "rust-lsp-mcp");
}

#[test]
fn tools_list_exposes_all_twenty_tools() {
    let mut client = Client::spawn(&common::fixture("simple"));
    let response = client.request("tools/list", Some(json!({})));
    let tools = response["result"]["tools"]
        .as_array()
        .expect("tools array");

    let names: BTreeSet<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    let expected: BTreeSet<&str> = [
        "hover",
        "definition",
        "references",
        "completion",
        "symbols",
        "format",
        "code_actions",
        "rename",
        "diagnostics",
        "workspace_diagnostics",
        "set_workspace",
        "implementation",
        "type_definition",
        "workspace_symbols",
        "inlay_hints",
        "signature_help",
        "expand_macro",
        "write_file",
        "read_file",
        "apply_edit",
    ]
    .into_iter()
    .collect();

    assert_eq!(names, expected);
}

#[test]
fn unknown_method_returns_method_not_found() {
    let mut client = Client::spawn(&common::fixture("simple"));
    let response = client.request("does/not/exist", Some(json!({})));
    assert_eq!(response["error"]["code"], -32601);
}

#[test]
fn unknown_tool_returns_internal_error() {
    let mut client = Client::spawn(&common::fixture("simple"));
    let response = client.request(
        "tools/call",
        Some(json!({ "name": "no_such_tool", "arguments": {} })),
    );
    assert_eq!(response["error"]["code"], -32000);
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Unknown tool")
    );
}

#[test]
fn tools_call_without_params_returns_invalid_params() {
    let mut client = Client::spawn(&common::fixture("simple"));
    let response = client.request("tools/call", None);
    assert_eq!(response["error"]["code"], -32602);
}
