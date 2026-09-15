//! rust-analyzer subprocess management and LSP request dispatch.
//!
//! # Content hash fix
//! The original used `std::collections::hash_map::DefaultHasher`, whose output
//! is intentionally unspecified across Rust versions and is not collision-resistant.
//! We use a simple FNV-1a 64-bit hash instead: deterministic, zero dependencies,
//! and good enough for change detection on source files.
//!
//! # find_rust_analyzer fix
//! The original fell back to a second `which::which` call that was guaranteed to
//! fail (if it had succeeded, the first call would have too). Now it returns `Err`
//! directly when the fallback path does not exist.

use anyhow::{anyhow, Result};
use log::{debug, info};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    process::ExitStatus,
    sync::Arc,
    time::Duration,
};
use std::process::Stdio;
use tokio::{
    io::BufWriter,
    process::{Child, Command},
    sync::{oneshot, watch, Mutex},
    task::JoinHandle,
};

use crate::{
    constants::{
        DOCUMENT_OPEN_DELAY_MS, GRACEFUL_SHUTDOWN_TIMEOUT_SECS, LSP_REQUEST_TIMEOUT_SECS,
        REQUEST_RETRY_LIMIT,
    },
    protocol::LspRequest,
    uri,
};

use super::connection::{send_message, Answer, Connection, Diagnostics, Flycheck, Outgoing, Pending};

pub struct RustAnalyzerClient {
    pub(super) process: Option<Child>,
    pub(super) request_id: u64,
    pub(super) workspace_root: PathBuf,
    pub(super) stdin: Option<Outgoing<tokio::process::ChildStdin>>,
    pub(super) pending: Pending,
    pub(super) initialized: bool,
    pub(super) open_docs: Arc<Mutex<HashMap<String, DocState>>>,
    pub(super) diagnostics: Diagnostics,
    pub(super) quiescent: watch::Sender<bool>,
    pub(super) flycheck: watch::Sender<Flycheck>,
    /// Set once we observe that rust-analyzer never reports cargo checks.
    pub(super) gave_up_on_checks: bool,
    /// URIs for which `didSave` has already been sent since the last edit.
    pub(super) saved_docs: std::collections::HashSet<String>,
    pub(super) reader: Option<JoinHandle<()>>,
    pub(super) settings: Value,
}

pub(super) struct DocState {
    pub version: u64,
    /// FNV-1a hash of the content last sent.
    pub content_hash: u64,
}

impl RustAnalyzerClient {
    pub fn new(workspace_root: PathBuf, settings: Value) -> Self {
        let workspace_root = uri::absolute(&workspace_root);
        Self {
            process: None,
            request_id: 1,
            workspace_root,
            stdin: None,
            pending: Arc::new(Mutex::new(HashMap::new())),
            initialized: false,
            open_docs: Arc::new(Mutex::new(HashMap::new())),
            diagnostics: Arc::new(Mutex::new(HashMap::new())),
            quiescent: watch::channel(false).0,
            flycheck: watch::channel(Flycheck::default()).0,
            gave_up_on_checks: false,
            saved_docs: std::collections::HashSet::new(),
            reader: None,
            settings,
        }
    }

    pub async fn start(&mut self) -> Result<()> {
        info!(
            "Starting rust-analyzer in: {}",
            self.workspace_root.display()
        );

        self.diagnostics.lock().await.clear();

        let ra_path = find_rust_analyzer()?;
        info!("rust-analyzer: {}", ra_path.display());

        let mut cmd = Command::new(&ra_path);
        cmd.current_dir(&self.workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        // Forward isolation variables when present.
        for var in ["XDG_CACHE_HOME", "CARGO_TARGET_DIR", "TMPDIR"] {
            if let Ok(val) = std::env::var(var) {
                cmd.env(var, val);
            }
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn rust-analyzer: {}", e))?;

        let stdin = child.stdin.take().ok_or_else(|| anyhow!("No stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("No stdout"))?;
        let stderr = child.stderr.take().ok_or_else(|| anyhow!("No stderr"))?;

        let stdin = Arc::new(Mutex::new(BufWriter::new(stdin)));
        self.stdin = Some(Arc::clone(&stdin));
        self.pending = Arc::new(Mutex::new(HashMap::new()));

        self.reader = Some(super::connection::spawn_io_tasks(
            stdout,
            stderr,
            Connection {
                pending_requests: Arc::clone(&self.pending),
                diagnostics: Arc::clone(&self.diagnostics),
                quiescent: self.quiescent.clone(),
                flycheck: self.flycheck.clone(),
                outgoing: stdin,
                settings: self.settings.clone(),
            },
        ));

        self.process = Some(child);
        self.initialize().await?;
        self.initialized = true;

        // Prompt rust-analyzer to apply our settings.
        let _ = self
            .send_notification(
                "workspace/didChangeConfiguration",
                Some(json!({ "settings": { "rust-analyzer": self.settings } })),
            )
            .await;

        info!("rust-analyzer ready");
        Ok(())
    }

    async fn initialize(&mut self) -> Result<()> {
        let init_params = json!({
            "processId": std::process::id(),
            "rootUri": uri::path_to_uri(&self.workspace_root)?,
            "initializationOptions": self.settings,
            "capabilities": {
                "textDocument": {
                    "hover": { "contentFormat": ["markdown", "plaintext"] },
                    "completion": { "completionItem": { "snippetSupport": true } },
                    "definition": { "linkSupport": true },
                    "implementation": { "linkSupport": true },
                    "typeDefinition": { "linkSupport": true },
                    "references": {},
                    "documentSymbol": {},
                    "signatureHelp": {
                        "signatureInformation": {
                            "documentationFormat": ["markdown", "plaintext"],
                            "parameterInformation": { "labelOffsetSupport": true }
                        }
                    },
                    "inlayHint": { "dynamicRegistration": false },
                    "codeAction": {
                        "codeActionLiteralSupport": {
                            "codeActionKind": {
                                "valueSet": [
                                    "quickfix", "refactor", "refactor.extract",
                                    "refactor.inline", "refactor.rewrite",
                                    "source", "source.organizeImports"
                                ]
                            }
                        },
                        "resolveSupport": { "properties": ["edit"] }
                    },
                    "publishDiagnostics": {
                        "relatedInformation": true,
                        "tagSupport": { "valueSet": [1, 2] }
                    },
                    "formatting": {},
                    "rename": {
                        "dynamicRegistration": false,
                        "prepareSupport": true
                    }
                },
                "workspace": {
                    "didChangeConfiguration": { "dynamicRegistration": false },
                    "workspaceEdit": {
                        "documentChanges": true,
                        "resourceOperations": ["create", "rename", "delete"],
                        "failureHandling": "abort"
                    },
                    "symbol": { "dynamicRegistration": false }
                },
                "general": { "positionEncodings": ["utf-16"] },
                "window": { "workDoneProgress": true },
                "experimental": { "serverStatusNotification": true }
            }
        });

        self.send_request("initialize", Some(init_params)).await?;
        self.send_notification("initialized", Some(json!({}))).await?;
        // Trigger an initial workspace load / cargo check.
        let _ = self.send_request("rust-analyzer/reloadWorkspace", None).await;
        Ok(())
    }

    /// Opens or updates a document in rust-analyzer's view of the workspace.
    ///
    /// Skips the network round-trip when the content hasn't changed since the last call
    /// (detected via FNV-1a hash). Holds back `didSave` while rust-analyzer is still
    /// loading to avoid a known panic in older versions.
    pub async fn open_document(&mut self, uri: &str, content: &str) -> Result<()> {
        let key = uri::normalize(uri);
        let hash = fnv1a(content);
        let known = self
            .open_docs
            .lock()
            .await
            .get(&key)
            .map(|s| (s.version, s.content_hash));

        match known {
            None => {
                debug!("Opening document: {}", uri);
                self.send_notification(
                    "textDocument/didOpen",
                    Some(json!({
                        "textDocument": {
                            "uri": uri,
                            "languageId": "rust",
                            "version": 1,
                            "text": content
                        }
                    })),
                )
                .await?;
                self.open_docs
                    .lock()
                    .await
                    .insert(key.clone(), DocState { version: 1, content_hash: hash });
            }
            Some((_, known_hash)) if known_hash == hash => {
                debug!("Document unchanged, skipping: {}", uri);
            }
            Some((version, _)) => {
                let version = version + 1;
                debug!("Updating document v{}: {}", version, uri);
                self.send_notification(
                    "textDocument/didChange",
                    Some(json!({
                        "textDocument": { "uri": uri, "version": version },
                        "contentChanges": [{ "text": content }]
                    })),
                )
                .await?;
                self.open_docs
                    .lock()
                    .await
                    .insert(key.clone(), DocState { version, content_hash: hash });
                // Stale diagnostics from before this edit are no longer meaningful.
                self.diagnostics.lock().await.remove(&key);
                self.saved_docs.remove(&key);
            }
        }

        // Send didSave only once per edit, and only when rust-analyzer is quiescent.
        // Sending it during workspace load can cause a panic in rust-analyzer <= 1.98.
        if self.saved_docs.contains(&key) {
            return Ok(());
        }
        if !*self.quiescent.borrow() {
            debug!("rust-analyzer busy; deferring didSave for {}", uri);
            return Ok(());
        }

        self.diagnostics.lock().await.remove(&key);
        self.send_notification(
            "textDocument/didSave",
            Some(json!({ "textDocument": { "uri": uri } })),
        )
        .await?;
        self.saved_docs.insert(key);

        // Give rust-analyzer a moment to register the save before the next request.
        tokio::time::sleep(Duration::from_millis(DOCUMENT_OPEN_DELAY_MS)).await;
        Ok(())
    }

    pub async fn close_document(&mut self, uri: &str) -> Result<()> {
        let key = uri::normalize(uri);
        if self.open_docs.lock().await.remove(&key).is_none() {
            return Ok(());
        }
        debug!("Closing document: {}", uri);
        self.send_notification(
            "textDocument/didClose",
            Some(json!({ "textDocument": { "uri": uri } })),
        )
        .await?;
        self.saved_docs.remove(&key);
        self.diagnostics.lock().await.remove(&key);
        Ok(())
    }

    pub async fn open_document_uris(&self) -> Vec<String> {
        self.open_docs.lock().await.keys().cloned().collect()
    }

    /// Shuts rust-analyzer down gracefully, then kills the process.
    pub async fn shutdown(&mut self) -> Result<()> {
        if self.initialized {
            let handshake = async {
                let _ = self.send_request("shutdown", None).await;
                let _ = self.send_notification("exit", None).await;
            };
            if tokio::time::timeout(
                Duration::from_secs(GRACEFUL_SHUTDOWN_TIMEOUT_SECS),
                handshake,
            )
            .await
            .is_err()
            {
                info!("Graceful shutdown timed out");
            }
        }
        self.force_kill().await;
        Ok(())
    }

    pub async fn force_kill(&mut self) {
        if let Some(mut p) = self.process.take() {
            let _ = p.kill().await;
            let _ = p.wait().await;
        }
        self.open_docs.lock().await.clear();
        self.saved_docs.clear();
        self.diagnostics.lock().await.clear();
        self.flycheck.send_replace(Flycheck::default());
        self.gave_up_on_checks = false;
        self.initialized = false;
    }

    pub fn is_gone(&self) -> bool {
        self.reader.as_ref().is_some_and(JoinHandle::is_finished)
    }

    pub fn exit_status(&mut self) -> Option<ExitStatus> {
        self.process.as_mut()?.try_wait().ok().flatten()
    }

    // -----------------------------------------------------------------------
    // Internal request / notification helpers
    // -----------------------------------------------------------------------

    pub(super) async fn send_notification(&mut self, method: &str, params: Option<Value>) -> Result<()> {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params.unwrap_or(json!({}))
        });
        let Some(stdin) = &self.stdin else {
            return Err(anyhow!("No stdin"));
        };
        send_message(stdin, &msg).await
    }

    /// Sends a request, retrying on `ContentModified` errors (rust-analyzer
    /// cancels in-flight requests when its analysis is superseded).
    pub(super) async fn send_request(&mut self, method: &str, params: Option<Value>) -> Result<Value> {
        for attempt in 1..REQUEST_RETRY_LIMIT {
            let result = self.send_request_once(method, params.clone()).await;
            match &result {
                Err(e) if is_content_modified(e) => {
                    debug!("Retrying {} (attempt {}): {}", method, attempt, e);
                }
                _ => return result,
            }
        }
        self.send_request_once(method, params).await
    }

    async fn send_request_once(&mut self, method: &str, params: Option<Value>) -> Result<Value> {
        let id = self.request_id;
        self.request_id += 1;

        let request = serde_json::to_value(LspRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params: params.clone(),
        })?;

        debug!("LSP request {}: {}", id, method);

        let (tx, rx) = oneshot::channel::<Answer>();
        self.pending.lock().await.insert(id, tx);

        let Some(stdin) = &self.stdin else {
            self.pending.lock().await.remove(&id);
            return Err(anyhow!("No stdin"));
        };

        if let Err(e) = send_message(stdin, &request).await {
            self.pending.lock().await.remove(&id);
            return Err(e);
        }

        match tokio::time::timeout(Duration::from_secs(LSP_REQUEST_TIMEOUT_SECS), rx).await {
            Ok(Ok(answer)) => answer.map_err(|m| anyhow!("{}", m)),
            Ok(Err(_)) => Err(anyhow!("rust-analyzer exited before responding")),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(anyhow!("LSP request timed out: {}", method))
            }
        }
    }
}

fn is_content_modified(e: &anyhow::Error) -> bool {
    e.to_string().to_lowercase().contains("content modified")
}

/// FNV-1a 64-bit hash — deterministic and zero extra dependencies.
fn fnv1a(s: &str) -> u64 {
    const OFFSET: u64 = 14695981039346656037;
    const PRIME: u64 = 1099511628211;
    s.bytes().fold(OFFSET, |h, b| (h ^ b as u64).wrapping_mul(PRIME))
}

fn find_rust_analyzer() -> Result<PathBuf> {
    if let Ok(path) = which::which("rust-analyzer") {
        return Ok(path);
    }
    // Common fallback: ~/.cargo/bin/rust-analyzer
    if let Ok(home) = std::env::var("HOME") {
        let fallback = PathBuf::from(home).join(".cargo/bin/rust-analyzer");
        if fallback.exists() {
            return Ok(fallback);
        }
    }
    Err(anyhow!(
        "rust-analyzer not found in PATH or ~/.cargo/bin. \
         Install it with: rustup component add rust-analyzer"
    ))
}
