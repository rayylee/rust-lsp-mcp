//! High-level LSP operations exposed to the MCP tool handlers.
//!
//! # workspace_diagnostics fix
//! `workspace/diagnostic` (pull model) returns `{ "kind": "full", "items": [...] }`,
//! which IS a JSON object. The old code checked `!result.is_object()` and fell through
//! to the wrong branch. We now check for the `"items"` field first regardless of
//! whether the outer value is an object, then fall back to the per-URI push map.
//!
//! # fresh_diagnostics
//! Waits for a cargo check to complete before returning, so diagnostics reflect
//! the file as it stands rather than as it was at the last check.

use anyhow::{anyhow, Result};
use log::{info, warn};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::watch;

use super::{client::RustAnalyzerClient, connection::Flycheck};
use crate::{
    constants::{
        DOCUMENT_OPEN_DELAY_MS, FLYCHECK_REQUEST_ATTEMPTS, FLYCHECK_START_TIMEOUT_SECS,
        FLYCHECK_TIMEOUT_SECS, WORKSPACE_LOAD_TIMEOUT_SECS,
    },
    uri,
};

/// Diagnostics for one file, plus a flag indicating whether the cargo check finished.
pub struct FreshDiagnostics {
    pub items: Value,
    /// False when the workspace was still loading or the check didn't finish —
    /// meaning these might be incomplete.
    pub complete: bool,
}

// The source label rust-analyzer puts on its own analysis results.
const ANALYSIS_SOURCE: &str = "rust-analyzer";

impl RustAnalyzerClient {
    // ------------------------------------------------------------------
    // Core LSP calls
    // ------------------------------------------------------------------

    pub async fn hover(&mut self, uri: &str, line: u32, character: u32) -> Result<Value> {
        self.send_request(
            "textDocument/hover",
            Some(json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            })),
        )
        .await
    }

    pub async fn definition(&mut self, uri: &str, line: u32, character: u32) -> Result<Value> {
        self.send_request(
            "textDocument/definition",
            Some(json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            })),
        )
        .await
    }

    pub async fn references(&mut self, uri: &str, line: u32, character: u32) -> Result<Value> {
        self.send_request(
            "textDocument/references",
            Some(json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character },
                "context": { "includeDeclaration": true }
            })),
        )
        .await
    }

    pub async fn completion(&mut self, uri: &str, line: u32, character: u32) -> Result<Value> {
        self.send_request(
            "textDocument/completion",
            Some(json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            })),
        )
        .await
    }

    pub async fn document_symbols(&mut self, uri: &str) -> Result<Value> {
        self.send_request(
            "textDocument/documentSymbol",
            Some(json!({ "textDocument": { "uri": uri } })),
        )
        .await
    }

    pub async fn formatting(&mut self, uri: &str) -> Result<Value> {
        self.send_request(
            "textDocument/formatting",
            Some(json!({
                "textDocument": { "uri": uri },
                "options": { "tabSize": 4, "insertSpaces": true }
            })),
        )
        .await
    }

    pub async fn code_actions(
        &mut self,
        uri: &str,
        start_line: u32,
        start_char: u32,
        end_line: u32,
        end_char: u32,
    ) -> Result<Value> {
        let diagnostics = self.published_diagnostics(uri).await;
        let filtered = filter_diagnostics_in_range(&json!(diagnostics), start_line, end_line);

        self.send_request(
            "textDocument/codeAction",
            Some(json!({
                "textDocument": { "uri": uri },
                "range": {
                    "start": { "line": start_line, "character": start_char },
                    "end":   { "line": end_line,   "character": end_char   }
                },
                "context": {
                    "diagnostics": filtered,
                    "only": [
                        "quickfix", "refactor", "refactor.extract",
                        "refactor.inline", "refactor.rewrite", "source"
                    ]
                }
            })),
        )
        .await
    }

    pub async fn implementation(&mut self, uri: &str, line: u32, character: u32) -> Result<Value> {
        self.send_request(
            "textDocument/implementation",
            Some(json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            })),
        )
        .await
    }

    pub async fn type_definition(&mut self, uri: &str, line: u32, character: u32) -> Result<Value> {
        self.send_request(
            "textDocument/typeDefinition",
            Some(json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            })),
        )
        .await
    }

    pub async fn workspace_symbols(&mut self, query: &str) -> Result<Value> {
        self.send_request(
            "workspace/symbol",
            Some(json!({ "query": query })),
        )
        .await
    }

    pub async fn inlay_hints(&mut self, uri: &str, start_line: u32, end_line: u32) -> Result<Value> {
        self.send_request(
            "textDocument/inlayHint",
            Some(json!({
                "textDocument": { "uri": uri },
                "range": {
                    "start": { "line": start_line, "character": 0 },
                    "end":   { "line": end_line,   "character": u32::MAX }
                }
            })),
        )
        .await
    }

    pub async fn expand_macro(&mut self, uri: &str, line: u32, character: u32) -> Result<Value> {
        self.send_request(
            "rust-analyzer/expandMacro",
            Some(json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            })),
        )
        .await
    }

    pub async fn signature_help(&mut self, uri: &str, line: u32, character: u32) -> Result<Value> {
        self.send_request(
            "textDocument/signatureHelp",
            Some(json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            })),
        )
        .await
    }

    pub async fn prepare_rename(&mut self, uri: &str, line: u32, character: u32) -> Result<Value> {
        self.send_request(
            "textDocument/prepareRename",
            Some(json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            })),
        )
        .await
    }

    pub async fn rename(
        &mut self,
        uri: &str,
        line: u32,
        character: u32,
        new_name: &str,
    ) -> Result<Value> {
        self.send_request(
            "textDocument/rename",
            Some(json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character },
                "newName": new_name
            })),
        )
        .await
    }

    // ------------------------------------------------------------------
    // Diagnostics
    // ------------------------------------------------------------------

    /// Waits for a fresh cargo check and returns its results for `uri`.
    pub async fn fresh_diagnostics(&mut self, uri: &str) -> Result<FreshDiagnostics> {
        let loaded = self
            .wait_until_loaded(Duration::from_secs(WORKSPACE_LOAD_TIMEOUT_SECS))
            .await;
        if !loaded {
            warn!("Workspace still loading; diagnostics may be incomplete");
        }

        let before = self.flycheck.borrow().clone();
        let mut progress = self.flycheck.subscribe();

        // Clear stale diagnostics so we always report from the new check.
        self.diagnostics.lock().await.clear();

        let params = json!({ "textDocument": { "uri": uri } });
        self.send_notification("rust-analyzer/runFlycheck", Some(params.clone()))
            .await?;

        // Short-circuit if we already know no checks will come.
        if self.gave_up_on_checks && before.never_ran_one() {
            return Ok(FreshDiagnostics {
                items: self.current_diagnostics(uri).await?,
                complete: false,
            });
        }

        // Wait for the check to start, retrying the request a few times because
        // rust-analyzer may drop it when its analysis is superseded mid-flight.
        let mut started = false;
        for attempt in 1..=FLYCHECK_REQUEST_ATTEMPTS {
            started = wait_for_flycheck(
                &mut progress,
                Duration::from_secs(FLYCHECK_START_TIMEOUT_SECS),
                |f| f.started_since(&before),
            )
            .await;
            if started || attempt == FLYCHECK_REQUEST_ATTEMPTS {
                break;
            }
            info!("Retrying cargo check request for {} (attempt {})", uri, attempt);
            self.send_notification("rust-analyzer/runFlycheck", Some(params.clone()))
                .await?;
        }

        if !started {
            if self.flycheck.borrow().never_ran_one() {
                warn!("rust-analyzer has not reported any cargo check; skipping wait");
                self.gave_up_on_checks = true;
            }
            return Ok(FreshDiagnostics {
                items: self.current_diagnostics(uri).await?,
                complete: false,
            });
        }

        let finished = wait_for_flycheck(
            &mut progress,
            Duration::from_secs(FLYCHECK_TIMEOUT_SECS),
            |f| f.caught_up_with(&before),
        )
        .await;
        if !finished {
            warn!("cargo check still running; returning early results");
        }

        // rust-analyzer publishes diagnostics just after reporting the check done.
        tokio::time::sleep(Duration::from_millis(DOCUMENT_OPEN_DELAY_MS)).await;

        Ok(FreshDiagnostics {
            items: self.current_diagnostics(uri).await?,
            complete: loaded && finished,
        })
    }

    /// Collects diagnostics for the whole workspace.
    ///
    /// Tries the LSP pull model (`workspace/diagnostic`) first.
    /// Falls back to re-running `diagnostics()` for each open document.
    ///
    /// # workspace/diagnostic format fix
    /// The pull model returns `{ "kind": "full", "items": [...] }`.
    /// We check for `"items"` regardless of nesting rather than relying on
    /// the value's shape.
    pub async fn workspace_diagnostics(&mut self) -> Result<Value> {
        if self.is_gone() {
            return Err(anyhow!("rust-analyzer is not running"));
        }

        let pull_result = self
            .send_request(
                "workspace/diagnostic",
                Some(json!({ "identifier": "rust-analyzer", "previousResultId": null })),
            )
            .await;

        match pull_result {
            Ok(resp) => {
                // Pull model: `{ "kind": "full", "items": [...] }`.
                // The response IS an object, so we look for "items" first.
                if let Some(items) = resp.get("items") {
                    return Ok(json!({ "kind": "full", "items": items }));
                }
                // Unexpected shape — return it as-is.
                Ok(resp)
            }
            Err(_) => {
                // Fallback: per-URI push diagnostics for all open documents.
                let uris: Vec<String> =
                    self.open_docs.lock().await.keys().cloned().collect();
                let mut map = serde_json::Map::new();
                for uri in uris {
                    let diags = self
                        .diagnostics
                        .lock()
                        .await
                        .get(&uri)
                        .cloned()
                        .unwrap_or_default();
                    map.insert(uri, json!(diags));
                }
                Ok(Value::Object(map))
            }
        }
    }

    /// Waits until rust-analyzer reports it is quiescent (no background work in flight).
    pub async fn wait_until_loaded(&self, timeout: Duration) -> bool {
        let mut rx = self.quiescent.subscribe();
        let wait = async {
            loop {
                if *rx.borrow_and_update() {
                    return true;
                }
                if rx.changed().await.is_err() {
                    return false;
                }
            }
        };
        tokio::time::timeout(timeout, wait).await.unwrap_or(false)
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// Merges rust-analyzer's own analysis with the latest cargo check results.
    async fn current_diagnostics(&mut self, uri: &str) -> Result<Value> {
        let published = self.published_diagnostics(uri).await;

        let params = json!({ "textDocument": { "uri": uri } });
        let pulled = match self.send_request("textDocument/diagnostic", Some(params)).await {
            Ok(v) => v,
            Err(e) => {
                warn!("Pull diagnostics failed for {}: {}", uri, e);
                return Ok(json!(published));
            }
        };

        let Some(analysed) = pulled.get("items").and_then(|i| i.as_array()) else {
            return Ok(json!(published));
        };

        // Keep cargo diagnostics from the push store, use fresh analysis diagnostics.
        let from_cargo = published
            .iter()
            .filter(|d| d["source"] != ANALYSIS_SOURCE)
            .cloned();

        Ok(json!(analysed
            .iter()
            .cloned()
            .chain(from_cargo)
            .collect::<Vec<_>>()))
    }

    async fn published_diagnostics(&self, uri: &str) -> Vec<Value> {
        self.diagnostics
            .lock()
            .await
            .get(&uri::normalize(uri))
            .cloned()
            .unwrap_or_default()
    }
}

async fn wait_for_flycheck(
    rx: &mut watch::Receiver<Flycheck>,
    timeout: Duration,
    cond: impl Fn(&Flycheck) -> bool,
) -> bool {
    let wait = async {
        loop {
            if cond(&rx.borrow_and_update()) {
                return true;
            }
            if rx.changed().await.is_err() {
                return false;
            }
        }
    };
    tokio::time::timeout(timeout, wait).await.unwrap_or(false)
}

fn filter_diagnostics_in_range(diagnostics: &Value, start_line: u32, end_line: u32) -> Value {
    let Some(arr) = diagnostics.as_array() else {
        return json!([]);
    };
    let filtered: Vec<Value> = arr
        .iter()
        .filter(|d| {
            let start = d.pointer("/range/start/line").and_then(|l| l.as_u64());
            let end = d.pointer("/range/end/line").and_then(|l| l.as_u64());
            match (start, end) {
                (Some(s), Some(e)) => s as u32 <= end_line && e as u32 >= start_line,
                _ => false,
            }
        })
        .cloned()
        .collect();
    json!(filtered)
}
