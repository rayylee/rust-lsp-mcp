use anyhow::Result;
use log::{debug, error, info, warn};
use serde_json::json;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};

use crate::{
    lsp::RustAnalyzerClient,
    protocol::{
        McpError, McpRequest, McpResponse, ERR_INTERNAL, ERR_INVALID_PARAMS, ERR_METHOD_NOT_FOUND,
    },
    settings::Settings,
    uri,
};

pub struct McpServer {
    pub(crate) client: Option<RustAnalyzerClient>,
    pub(crate) workspace_root: PathBuf,
    settings: Settings,
}

impl McpServer {
    pub fn new(workspace_root: PathBuf, settings: Settings) -> Self {
        Self {
            client: None,
            workspace_root: uri::absolute(&workspace_root),
            settings,
        }
    }

    /// Ensures rust-analyzer is running, restarting it if it has crashed.
    pub async fn ensure_client(&mut self) -> Result<()> {
        if let Some(client) = &mut self.client {
            if client.is_gone() {
                match client.exit_status() {
                    Some(status) => warn!("rust-analyzer exited ({}); restarting", status),
                    None => warn!("rust-analyzer connection closed; restarting"),
                }
                self.client = None;
            }
        }

        if self.client.is_none() {
            let settings_json = self.settings.to_json();
            let mut client = RustAnalyzerClient::new(self.workspace_root.clone(), settings_json);
            client.start().await?;
            self.client = Some(client);
        }
        Ok(())
    }

    /// Returns a mutable reference to the active client.
    /// Call `ensure_client` before this.
    pub(crate) fn lsp(&mut self) -> &mut RustAnalyzerClient {
        self.client.as_mut().expect("ensure_client not called")
    }

    /// Reads `file_path` from disk and syncs its content into rust-analyzer.
    /// Returns the file's `file://` URI for use in subsequent LSP calls.
    pub(crate) async fn load_file(&mut self, file_path: &str) -> Result<String> {
        let path = self.resolve_path(file_path);
        let uri = uri::path_to_uri(&path)?;
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| anyhow::anyhow!("Cannot read {}: {}", path.display(), e))?;
        self.lsp().open_document(&uri, &content).await?;
        Ok(uri)
    }

    /// Re-reads every open document from disk so rust-analyzer's view is current.
    /// Used before operations that span the whole workspace (e.g. rename).
    pub(crate) async fn refresh_open_docs(&mut self) -> Result<()> {
        let uris = self.lsp().open_document_uris().await;
        for uri in uris {
            let Some(path) = uri::uri_to_path(&uri) else {
                continue;
            };
            match tokio::fs::read_to_string(&path).await {
                Ok(content) => self.lsp().open_document(&uri, &content).await?,
                Err(_) => self.lsp().close_document(&uri).await?,
            }
        }
        Ok(())
    }

    /// Resolves a file path argument to an absolute `PathBuf`.
    /// Accepts relative paths (joined against workspace root), absolute paths, and `file://` URIs.
    pub(crate) fn resolve_path(&self, file_path: &str) -> PathBuf {
        let path = match uri::uri_to_path(file_path) {
            Some(p) => p,
            None => self.workspace_root.join(file_path),
        };
        uri::absolute(&path)
    }

    // ------------------------------------------------------------------
    // Server main loop
    // ------------------------------------------------------------------

    pub async fn run(&mut self) -> Result<()> {
        info!("rust-dev-mcp starting");

        let mut reader = BufReader::new(tokio::io::stdin());
        let mut writer = BufWriter::new(tokio::io::stdout());
        let mut shutdown = ShutdownSignal::new()?;
        let mut result = Ok(());

        'main: loop {
            let mut line = String::new();

            let bytes = tokio::select! {
                biased;
                _ = shutdown.recv() => {
                    info!("Shutdown signal received");
                    break 'main;
                }
                read = reader.read_line(&mut line) => match read {
                    Ok(n) => n,
                    Err(e) => {
                        error!("stdin read error: {}", e);
                        result = Err(e.into());
                        break 'main;
                    }
                }
            };

            if bytes == 0 {
                break; // EOF
            }

            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let Ok(request) = serde_json::from_str::<McpRequest>(line) else {
                debug!("Ignoring unparseable message: {}", line);
                continue;
            };

            // Notifications (no id) must never be answered.
            if request.id.is_none() {
                debug!("Notification: {}", request.method);
                continue;
            }

            let response = tokio::select! {
                biased;
                _ = shutdown.recv() => {
                    info!("Shutdown signal received mid-request");
                    break 'main;
                }
                r = self.handle(&request) => r,
            };

            let json = match serde_json::to_string(&response) {
                Ok(s) => s,
                Err(e) => {
                    error!("Serialization error: {}", e);
                    result = Err(e.into());
                    break 'main;
                }
            };

            let write = async {
                writer.write_all(json.as_bytes()).await?;
                writer.write_all(b"\n").await?;
                writer.flush().await
            };

            let written = tokio::select! {
                biased;
                _ = shutdown.recv() => break 'main,
                w = write => w,
            };
            if let Err(e) = written {
                error!("stdout write error: {}", e);
                result = Err(e.into());
                break 'main;
            }
        }

        // Graceful rust-analyzer shutdown.
        info!("Shutting down");
        if let Some(client) = &mut self.client {
            client.shutdown().await.ok();
        }

        result
    }

    async fn handle(&mut self, request: &McpRequest) -> McpResponse {
        let id = request.id.clone();

        match request.method.as_str() {
            "initialize" => McpResponse::Success {
                jsonrpc: "2.0".into(),
                id,
                result: json!({
                    "protocolVersion": "2024-11-05",
                    "serverInfo": {
                        "name": "rust-dev-mcp",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "capabilities": { "tools": {} }
                }),
            },
            "ping" => McpResponse::Success {
                jsonrpc: "2.0".into(),
                id,
                result: json!({}),
            },
            "tools/list" => McpResponse::Success {
                jsonrpc: "2.0".into(),
                id,
                result: json!({ "tools": super::tools::list() }),
            },
            "tools/call" => {
                let Some(params) = &request.params else {
                    return error_response(id, ERR_INVALID_PARAMS, "Missing params");
                };
                let Some(tool_name) = params["name"].as_str() else {
                    return error_response(id, ERR_INVALID_PARAMS, "Missing tool name");
                };
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or(json!({}));

                match super::handlers::dispatch(self, tool_name, args).await {
                    Ok(result) => McpResponse::Success {
                        jsonrpc: "2.0".into(),
                        id,
                        result: serde_json::to_value(result).unwrap(),
                    },
                    Err(e) => {
                        error!("Tool '{}' error: {}", tool_name, e);
                        error_response(id, ERR_INTERNAL, &e.to_string())
                    }
                }
            }
            method => error_response(
                id,
                ERR_METHOD_NOT_FOUND,
                &format!("Method not found: {}", method),
            ),
        }
    }
}

fn error_response(id: Option<serde_json::Value>, code: i32, message: &str) -> McpResponse {
    McpResponse::Error {
        jsonrpc: "2.0".into(),
        id,
        error: McpError {
            code,
            message: message.to_string(),
            data: None,
        },
    }
}

// ---------------------------------------------------------------------------
// Signal handling
// ---------------------------------------------------------------------------

struct ShutdownSignal {
    #[cfg(unix)]
    sigint: tokio::signal::unix::Signal,
    #[cfg(unix)]
    sigterm: tokio::signal::unix::Signal,
    #[cfg(unix)]
    sighup: tokio::signal::unix::Signal,
    #[cfg(windows)]
    ctrl_c: tokio::signal::windows::CtrlC,
    #[cfg(windows)]
    ctrl_close: tokio::signal::windows::CtrlClose,
}

impl ShutdownSignal {
    fn new() -> Result<Self> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            Ok(Self {
                sigint: signal(SignalKind::interrupt())?,
                sigterm: signal(SignalKind::terminate())?,
                sighup: signal(SignalKind::hangup())?,
            })
        }
        #[cfg(windows)]
        {
            use tokio::signal::windows;
            Ok(Self {
                ctrl_c: windows::ctrl_c()?,
                ctrl_close: windows::ctrl_close()?,
            })
        }
        #[cfg(not(any(unix, windows)))]
        Ok(Self {})
    }

    async fn recv(&mut self) {
        #[cfg(unix)]
        {
            tokio::select! {
                _ = self.sigint.recv()  => {}
                _ = self.sigterm.recv() => {}
                _ = self.sighup.recv()  => {}
            }
        }
        #[cfg(windows)]
        {
            tokio::select! {
                _ = self.ctrl_c.recv()    => {}
                _ = self.ctrl_close.recv() => {}
            }
        }
        #[cfg(not(any(unix, windows)))]
        std::future::pending::<()>().await
    }
}
