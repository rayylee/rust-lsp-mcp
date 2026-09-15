//! File I/O tool tests. These skip rust-analyzer entirely and run quickly.

mod common;

use common::Client;
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

fn temp_dir() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rust-dev-mcp-e2e-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn read_file_round_trips_fixture_content() {
    let mut client = Client::spawn(&common::fixture("simple"));
    let expected = common::fixture_file("simple", "src/lib.rs");
    let actual = client.call_tool_text("read_file", json!({ "file_path": "src/lib.rs" }));
    assert_eq!(actual, expected);
}

#[test]
fn write_file_creates_parent_directories() {
    let mut client = Client::spawn(&common::fixture("simple"));
    let dir = temp_dir();
    let path = dir.join("nested").join("inner").join("main.rs");
    let content = "fn main() {}\n";

    let result = client.call_tool_text(
        "write_file",
        json!({ "file_path": path.to_str().unwrap(), "content": content }),
    );
    assert!(result.contains("bytes"), "unexpected write result: {result}");
    assert_eq!(fs::read_to_string(&path).unwrap(), content);
}

#[test]
fn apply_edit_applies_unsorted_edits_correctly() {
    let mut client = Client::spawn(&common::fixture("simple"));
    let dir = temp_dir();
    let path = dir.join("main.rs");
    let original = "fn main() {\n    let a = 1;\n    let b = 2;\n}\n";
    fs::write(&path, original).unwrap();

    // Deliberately pass edits in reverse order; apply_edit must sort them.
    let edits = json!([
        {
            "range": {
                "start": { "line": 2, "character": 12 },
                "end": { "line": 2, "character": 13 }
            },
            "newText": "3"
        },
        {
            "range": {
                "start": { "line": 1, "character": 12 },
                "end": { "line": 1, "character": 13 }
            },
            "newText": "2"
        }
    ]);

    let result = client.call_tool_text(
        "apply_edit",
        json!({ "file_path": path.to_str().unwrap(), "edits": edits }),
    );
    assert!(result.contains("2 edit"), "unexpected apply result: {result}");

    let expected = "fn main() {\n    let a = 2;\n    let b = 3;\n}\n";
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
}
