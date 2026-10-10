//! Shared helpers for end-to-end tests.
//!
//! These tests drive the compiled `rust-lsp-mcp` binary over stdio exactly like
//! a real MCP client: newline-delimited JSON-RPC requests in, responses out.
//!
//! Each integration-test binary only uses a subset of these helpers, so silence
//! dead-code warnings per binary.

#![allow(dead_code)]

use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    time::{Duration, Instant},
};

/// Upper bound for a single MCP request to complete. Some tools (diagnostics,
/// rename) wait for cargo check and can take tens of seconds on a cold cache.
pub const TIMEOUT: Duration = Duration::from_secs(180);

pub struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    reader: BufReader<ChildStdout>,
    next_id: u64,
}

impl Client {
    /// Spawns the server pointed at `workspace`.
    pub fn spawn(workspace: &Path) -> Self {
        // Keep cargo build artifacts out of the checked-in fixture directories.
        let target_dir = std::env::temp_dir().join(format!(
            "rust-lsp-mcp-e2e-target-{}",
            std::process::id()
        ));

        let mut child = Command::new(env!("CARGO_BIN_EXE_rust-lsp-mcp"))
            .arg(workspace)
            .env("CARGO_TARGET_DIR", &target_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn rust-lsp-mcp");

        let stdin = child.stdin.take().expect("child stdin");
        let stdout = child.stdout.take().expect("child stdout");

        Self {
            child,
            stdin: Some(stdin),
            reader: BufReader::new(stdout),
            next_id: 1,
        }
    }

    /// Sends a JSON-RPC request and returns the response object for that id.
    pub fn request(&mut self, method: &str, params: Option<Value>) -> Value {
        let id = self.next_id;
        self.next_id += 1;

        let mut msg = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        if let Some(params) = params {
            msg["params"] = params;
        }

        let stdin = self.stdin.as_mut().expect("stdin already closed");
        writeln!(stdin, "{}", msg).expect("write request");
        stdin.flush().expect("flush request");

        let deadline = Instant::now() + TIMEOUT;
        loop {
            let mut line = String::new();
            let n = self
                .reader
                .read_line(&mut line)
                .expect("read response line");
            assert!(n > 0, "server closed stdout while waiting for {method}");

            let response: Value = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("non-JSON response line {line:?}: {e}"));
            if response["id"] == json!(id) {
                return response;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {method}");
        }
    }

    /// Calls an MCP tool and returns the text payload of its result.
    pub fn call_tool_text(&mut self, name: &str, args: Value) -> String {
        let response = self.request(
            "tools/call",
            Some(json!({ "name": name, "arguments": args })),
        );
        assert!(
            response.get("error").is_none(),
            "tool {name} returned an error: {response}"
        );
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("tool {name} result has no text: {response}"))
            .to_string()
    }

    /// Calls an MCP tool whose result text is JSON, and parses it.
    pub fn call_tool(&mut self, name: &str, args: Value) -> Value {
        let text = self.call_tool_text(name, args);
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("tool {name} returned non-JSON text {text:?}: {e}"))
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // Closing stdin makes the server hit EOF and shut down gracefully,
        // which also tears down its rust-analyzer child.
        drop(self.stdin.take());

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = self.child.kill();
                        let _ = self.child.wait();
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(_) => return,
            }
        }
    }
}

/// Absolute path to a checked-in fixture workspace.
pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// Reads a file inside a fixture workspace.
pub fn fixture_file(name: &str, relative: &str) -> String {
    std::fs::read_to_string(fixture(name).join(relative)).expect("read fixture file")
}

/// 0-based (line, UTF-16 character) of the first occurrence of `marker`.
pub fn pos_of(source: &str, marker: &str) -> (u32, u32) {
    let byte = source
        .find(marker)
        .unwrap_or_else(|| panic!("marker {marker:?} not found"));
    line_col_at(source, byte)
}

/// 0-based (line, UTF-16 character) just after the first occurrence of `marker`.
pub fn pos_after(source: &str, marker: &str) -> (u32, u32) {
    let byte = source
        .find(marker)
        .unwrap_or_else(|| panic!("marker {marker:?} not found"))
        + marker.len();
    line_col_at(source, byte)
}

fn line_col_at(source: &str, byte: usize) -> (u32, u32) {
    let line = source[..byte].bytes().filter(|&b| b == b'\n').count() as u32;
    let line_start = source[..byte].rfind('\n').map_or(0, |i| i + 1);
    let character = source[line_start..byte].encode_utf16().count() as u32;
    (line, character)
}

/// Polls `hover` at `(line, character)` until rust-analyzer returns a result.
///
/// `hover` needs type inference and a loaded crate graph, so a non-null result
/// is a stronger readiness signal than document symbols (which can appear as
/// soon as a file is parsed). Most semantic tools are reliable after this.
pub fn wait_until_hover_ready(
    client: &mut Client,
    relative: &str,
    line: u32,
    character: u32,
) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let hover = client.call_tool(
            "hover",
            json!({ "file_path": relative, "line": line, "character": character }),
        );
        if !hover.is_null() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "rust-analyzer never indexed {relative}"
        );
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// Calls `tool` with `args` until `ready(result)` is true, then returns that
/// result. Some tools return null or an empty list while rust-analyzer's index
/// is still warming up even after hover succeeds, so poll rather than fail fast.
pub fn retry_tool_until(
    client: &mut Client,
    tool: &str,
    args: Value,
    ready: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let result = client.call_tool(tool, args.clone());
        if ready(&result) {
            return result;
        }
        assert!(
            Instant::now() < deadline,
            "{tool} never became ready; last result: {result}"
        );
        std::thread::sleep(Duration::from_millis(500));
    }
}

pub fn rust_analyzer_available() -> bool {
    which::which("rust-analyzer").is_ok()
        || std::env::var_os("HOME")
            .map(|home| Path::new(&home).join(".cargo/bin/rust-analyzer").exists())
            .unwrap_or(false)
}
