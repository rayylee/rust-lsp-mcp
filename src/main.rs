use anyhow::Result;
use rust_lsp_mcp::{
    cli::{self, Action, USAGE},
    mcp::McpServer,
};

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let (workspace, settings) = match cli::parse(std::env::args_os().skip(1))? {
        Action::Help => {
            print!("{USAGE}");
            return Ok(());
        }
        Action::Version => {
            println!("rust-lsp-mcp {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Action::Serve { workspace, settings } => (workspace, settings),
    };

    let workspace = workspace
        .unwrap_or_else(|| std::env::current_dir().expect("Failed to get current directory"));

    let mut server = McpServer::new(workspace, settings);

    let runtime = tokio::runtime::Runtime::new()?;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        runtime.block_on(server.run())
    }));

    // tokio's stdin reader parks an uncancellable blocking thread; shutdown_background
    // avoids hanging when the server exits via signal rather than stdin EOF.
    runtime.shutdown_background();

    match result {
        Ok(r) => r,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
