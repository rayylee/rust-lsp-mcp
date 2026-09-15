/// Timeout for individual LSP requests, in seconds.
pub const LSP_REQUEST_TIMEOUT_SECS: u64 = 30;

/// Graceful shutdown handshake budget, in seconds.
pub const GRACEFUL_SHUTDOWN_TIMEOUT_SECS: u64 = 5;

/// Delay after `didSave` to let rust-analyzer register the save, in milliseconds.
pub const DOCUMENT_OPEN_DELAY_MS: u64 = 200;

/// How long to wait for a cargo check to start after requesting one.
pub const FLYCHECK_START_TIMEOUT_SECS: u64 = 2;

/// How many times to retry a cargo check request that never starts.
pub const FLYCHECK_REQUEST_ATTEMPTS: usize = 5;

/// Budget for a running cargo check to complete.
pub const FLYCHECK_TIMEOUT_SECS: u64 = 30;

/// How long to wait for rust-analyzer to finish loading before a workspace-wide operation.
pub const WORKSPACE_LOAD_TIMEOUT_SECS: u64 = 30;

/// How many times to retry an LSP request cancelled with `ContentModified`.
pub const REQUEST_RETRY_LIMIT: u32 = 3;
