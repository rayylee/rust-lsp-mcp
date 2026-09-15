//! Async I/O bridge between this server and the rust-analyzer subprocess.
//!
//! # LSP framing fix
//! The LSP header section may contain multiple header fields
//! (e.g. `Content-Length` followed by `Content-Type`). The original code
//! skipped exactly one line after finding `Content-Length`, which was wrong
//! when `Content-Type` was also present. This implementation reads all header
//! lines until it sees the blank separator, then reads the body.
//!
//! # read_exact failure fix
//! A failed `read_exact` previously `continue`d, leaving the stream at an
//! unknown offset. We now `break` to tear down the connection cleanly.

use anyhow::Result;
use log::{debug, error, info, warn};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter},
    sync::{oneshot, watch, Mutex},
    task::JoinHandle,
};

use crate::{protocol::LspResponse, uri};

pub type Diagnostics = Arc<Mutex<HashMap<String, Vec<Value>>>>;
pub type Answer = std::result::Result<Value, String>;
pub type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Answer>>>>;
pub type Outgoing<W> = Arc<Mutex<BufWriter<W>>>;

/// Tracks cargo checks reported by rust-analyzer `$/progress` notifications.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Flycheck {
    running: HashSet<String>,
    started: u64,
}

impl Flycheck {
    pub fn begin(&mut self, token: &str) {
        self.running.insert(token.to_string());
        self.started += 1;
    }

    pub fn end(&mut self, token: &str) {
        self.running.remove(token);
    }

    pub fn caught_up_with(&self, earlier: &Self) -> bool {
        self.started > earlier.started && self.running.is_empty()
    }

    pub fn started_since(&self, earlier: &Self) -> bool {
        self.started > earlier.started
    }

    pub fn never_ran_one(&self) -> bool {
        self.started == 0
    }
}

const FLYCHECK_TOKEN: &str = "rust-analyzer/flycheck/";

pub struct Connection<W> {
    pub pending_requests: Pending,
    pub diagnostics: Diagnostics,
    pub quiescent: watch::Sender<bool>,
    pub flycheck: watch::Sender<Flycheck>,
    pub outgoing: Outgoing<W>,
    pub settings: Value,
}

/// Writes one LSP message with the `Content-Length` header.
pub async fn send_message<W: AsyncWrite + Unpin>(outgoing: &Outgoing<W>, message: &Value) -> Result<()> {
    let body = serde_json::to_string(message)?;
    let frame = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    let mut w = outgoing.lock().await;
    w.write_all(frame.as_bytes()).await?;
    w.flush().await?;
    Ok(())
}

/// Spawns background tasks that read rust-analyzer's stdout and stderr.
/// Returns the stdout-reader handle; it completes when rust-analyzer closes its pipe.
pub fn spawn_io_tasks<W: AsyncWrite + Unpin + Send + 'static>(
    stdout: impl AsyncRead + Unpin + Send + 'static,
    stderr: impl AsyncRead + Unpin + Send + 'static,
    connection: Connection<W>,
) -> JoinHandle<()> {
    tokio::spawn(log_stderr(stderr));
    tokio::spawn(read_stdout(stdout, connection))
}

async fn log_stderr(stderr: impl AsyncRead + Unpin) {
    let mut reader = BufReader::new(stderr);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    info!("rust-analyzer: {}", trimmed);
                }
            }
            Err(e) => {
                error!("stderr read error: {}", e);
                break;
            }
        }
    }
}

async fn read_stdout<W: AsyncWrite + Unpin>(stdout: impl AsyncRead + Unpin, connection: Connection<W>) {
    let mut reader = BufReader::new(stdout);

    'outer: loop {
        // --- Read all header lines until the blank separator ---
        let mut content_length: Option<usize> = None;
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line).await {
                Ok(0) => break 'outer, // EOF
                Ok(_) => {}
                Err(e) => {
                    error!("stdout read error: {}", e);
                    break 'outer;
                }
            }

            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                // Blank line: end of headers
                break;
            }
            // Case-insensitive header name matching per HTTP/LSP spec
            if let Some(rest) = trimmed.to_ascii_lowercase().strip_prefix("content-length:") {
                match rest.trim().parse::<usize>() {
                    Ok(n) => content_length = Some(n),
                    Err(_) => {
                        error!("Unparseable Content-Length: {:?}", trimmed);
                        break 'outer;
                    }
                }
            }
            // Other headers (e.g. Content-Type) are ignored but consumed here.
        }

        let Some(length) = content_length else {
            // No Content-Length seen before the blank line; skip.
            debug!("LSP message with no Content-Length, skipping");
            continue;
        };

        // --- Read exactly `length` bytes of body ---
        let mut body = vec![0u8; length];
        if let Err(e) = reader.read_exact(&mut body).await {
            // Stream is now at an indeterminate offset; we cannot recover.
            error!("Failed to read LSP body ({} bytes): {}", length, e);
            break;
        }

        debug!("LSP message body: {}", String::from_utf8_lossy(&body));
        dispatch_message(&body, &connection).await;
    }

    // Fail all outstanding requests so callers don't hang until their timeout.
    let mut pending = connection.pending_requests.lock().await;
    for (_, tx) in pending.drain() {
        let _ = tx.send(Err("rust-analyzer exited before responding".to_string()));
    }
}

async fn dispatch_message<W: AsyncWrite + Unpin>(body: &[u8], conn: &Connection<W>) {
    let Ok(msg) = serde_json::from_slice::<Value>(body) else {
        error!("JSON parse error: {}", String::from_utf8_lossy(body));
        return;
    };

    match (msg.get("method"), msg.get("id")) {
        // rust-analyzer → us: a request (has both method and id)
        (Some(method), Some(id)) => {
            let method = method.as_str().unwrap_or_default().to_string();
            answer_server_request(&method, id.clone(), &msg, conn).await;
        }
        // rust-analyzer → us: a notification (method, no id)
        (Some(_), None) => handle_notification(msg, conn).await,
        // us → rust-analyzer: a response to one of our requests
        (None, Some(_)) => deliver_response(msg, &conn.pending_requests).await,
        (None, None) => debug!("Ignoring LSP message with neither method nor id"),
    }
}

async fn deliver_response(msg: Value, pending: &Pending) {
    let Ok(resp) = serde_json::from_value::<LspResponse>(msg) else {
        return;
    };
    let Some(id) = resp.id else { return };

    let Some(tx) = pending.lock().await.remove(&id) else {
        return;
    };

    if let Some(error) = resp.error {
        let message = error
            .get("message")
            .and_then(|m| m.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| error.to_string());
        let _ = tx.send(Err(message));
    } else {
        let _ = tx.send(Ok(resp.result.unwrap_or(json!(null))));
    }
}

/// Answers a request that rust-analyzer made of us (e.g. `workspace/configuration`).
/// Every such request must be answered exactly once with the matching id.
async fn answer_server_request<W: AsyncWrite + Unpin>(
    method: &str,
    id: Value,
    request: &Value,
    conn: &Connection<W>,
) {
    debug!("rust-analyzer request: {}", method);

    let response = match method {
        "workspace/configuration" => {
            // Repeat our settings once per section asked about.
            let sections = request
                .pointer("/params/items")
                .and_then(Value::as_array)
                .map_or(1, Vec::len);
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": vec![conn.settings.clone(); sections],
            })
        }
        "window/workDoneProgress/create" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": null,
        }),
        _ => {
            info!("Declining unsupported rust-analyzer request: {}", method);
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32601,
                    "message": format!("Method not found: {method}"),
                },
            })
        }
    };

    if let Err(e) = send_message(&conn.outgoing, &response).await {
        error!("Failed to answer rust-analyzer '{}': {}", method, e);
    }
}

async fn handle_notification<W: AsyncWrite + Unpin>(msg: Value, conn: &Connection<W>) {
    let Some(method) = msg.get("method").and_then(|m| m.as_str()) else {
        return;
    };
    let Some(params) = msg.get("params") else { return };

    debug!("Notification: {}", method);

    match method {
        "textDocument/publishDiagnostics" => {
            let Some(uri) = params.get("uri").and_then(|u| u.as_str()) else {
                return;
            };
            let Some(diags) = params.get("diagnostics").and_then(|d| d.as_array()) else {
                return;
            };
            conn.diagnostics
                .lock()
                .await
                .insert(uri::normalize(uri), diags.clone());
            debug!("Stored {} diagnostics for {}", diags.len(), uri);
        }
        "experimental/serverStatus" => {
            let Some(quiescent) = params.get("quiescent").and_then(|q| q.as_bool()) else {
                return;
            };
            debug!("rust-analyzer quiescent: {}", quiescent);
            if let Some(msg) = params.get("message").and_then(|m| m.as_str()) {
                warn!("rust-analyzer status: {}", msg);
            }
            conn.quiescent.send_replace(quiescent);
        }
        "$/progress" => {
            let Some(token) = params.get("token").and_then(|t| t.as_str()) else {
                return;
            };
            if !token.starts_with(FLYCHECK_TOKEN) {
                return;
            }
            match params.pointer("/value/kind").and_then(|k| k.as_str()) {
                Some("begin") => {
                    debug!("cargo check started: {}", token);
                    conn.flycheck.send_modify(|f| f.begin(token));
                }
                Some("end") => {
                    debug!("cargo check finished: {}", token);
                    conn.flycheck.send_modify(|f| f.end(token));
                }
                _ => {}
            }
        }
        _ => {}
    }
}
