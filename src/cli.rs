//! Command-line argument parsing.

use anyhow::{bail, Result};
use std::{ffi::OsString, path::PathBuf};

use crate::settings::Settings;

pub const USAGE: &str = "\
MCP server bridging AI assistants to rust-analyzer.

Usage: rust-lsp-mcp [OPTIONS] [--] [WORKSPACE]

Arguments:
  [WORKSPACE]  Workspace root [default: current directory]

Options:
      --all-features         Analyse with every cargo feature enabled
      --features <LIST>      Comma- or space-separated features to enable (repeatable)
      --no-default-features  Omit manifest default features
      --config <KEY=VALUE>   Set any rust-analyzer option, e.g. --config check.command=clippy
  -h, --help                 Print this help
  -V, --version              Print version
";

pub enum Action {
    Help,
    Version,
    Serve {
        workspace: Option<PathBuf>,
        settings: Settings,
    },
}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Action> {
    let mut args = args.into_iter();
    let mut workspace = None;
    let mut settings = Settings::default();
    let mut opts_done = false;

    while let Some(arg) = args.next() {
        if !opts_done && arg == "--" {
            opts_done = true;
            continue;
        }

        let option = (!opts_done)
            .then(|| arg.to_str())
            .flatten()
            .filter(|a| a.starts_with('-'));

        if let Some(opt) = option {
            let (name, inline) = match opt.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (opt, None),
            };

            match name {
                "-h" | "--help" => return Ok(Action::Help),
                "-V" | "--version" => return Ok(Action::Version),
                "--all-features" => settings.enable_all_features()?,
                "--no-default-features" => settings.disable_default_features()?,
                "--features" => settings.enable_features(&value(name, inline, &mut args)?)?,
                "--config" => settings.set(&value(name, inline, &mut args)?)?,
                _ => bail!("Unknown option '{opt}'\n\n{USAGE}"),
            }
            continue;
        }

        if workspace.replace(PathBuf::from(&arg)).is_some() {
            bail!(
                "Unexpected extra argument '{}'\n\n{USAGE}",
                arg.to_string_lossy()
            );
        }
    }

    Ok(Action::Serve { workspace, settings })
}

fn value(
    name: &str,
    inline: Option<String>,
    args: &mut impl Iterator<Item = OsString>,
) -> Result<String> {
    if let Some(v) = inline {
        return Ok(v);
    }
    match args.next() {
        Some(v) => v.into_string().map_err(|_| {
            anyhow::anyhow!("{name}'s value is not valid UTF-8")
        }),
        None => bail!("{name} requires a value\n\n{USAGE}"),
    }
}
