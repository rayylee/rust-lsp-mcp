//! The JSON configuration object passed to rust-analyzer at startup.
//! rust-analyzer has no command line; every option is a JSON field.

use anyhow::{bail, Result};
use serde_json::{json, Value};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Features {
    #[default]
    Default,
    All,
    Named(Vec<String>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    features: Features,
    no_default_features: bool,
    overrides: Vec<(String, Value)>,
}

impl Settings {
    pub fn enable_all_features(&mut self) -> Result<()> {
        if self.features != Features::Default {
            bail!("--all-features cannot be combined with --features");
        }
        self.features = Features::All;
        Ok(())
    }

    pub fn enable_features(&mut self, list: &str) -> Result<()> {
        let named: Vec<String> = list
            .split([',', ' ', '\t'])
            .filter(|f| !f.is_empty())
            .map(str::to_string)
            .collect();
        if named.is_empty() {
            bail!("--features needs at least one feature name");
        }
        match &mut self.features {
            Features::All => bail!("--features cannot be combined with --all-features"),
            Features::Named(existing) => existing.extend(named),
            f @ Features::Default => *f = Features::Named(named),
        }
        Ok(())
    }

    pub fn disable_default_features(&mut self) -> Result<()> {
        if self.features == Features::All {
            bail!("--no-default-features cannot be combined with --all-features");
        }
        self.no_default_features = true;
        Ok(())
    }

    /// Accepts `key.path=value`, where value is JSON or a bare string.
    pub fn set(&mut self, assignment: &str) -> Result<()> {
        let Some((key, value)) = assignment.split_once('=') else {
            bail!("--config expects KEY=VALUE, e.g. check.command=clippy");
        };
        if key.is_empty() || key.split('.').any(str::is_empty) {
            bail!("'{key}' is not a valid setting path");
        }
        let value = serde_json::from_str(value).unwrap_or_else(|_| json!(value));
        self.overrides.push((key.to_string(), value));
        Ok(())
    }

    pub fn to_json(&self) -> Value {
        let mut s = json!({
            "cargo": { "buildScripts": { "enable": true } },
            "checkOnSave": true,
            "diagnostics": {
                "enable": true,
                "experimental": { "enable": true }
            },
            "procMacro": { "enable": true }
        });

        match &self.features {
            Features::Default => {}
            Features::All => s["cargo"]["features"] = json!("all"),
            Features::Named(fs) => s["cargo"]["features"] = json!(fs),
        }
        if self.no_default_features {
            s["cargo"]["noDefaultFeatures"] = json!(true);
        }
        for (key, value) in &self.overrides {
            set_nested(&mut s, key, value.clone());
        }
        s
    }
}

fn set_nested(root: &mut Value, key: &str, value: Value) {
    let mut at = root;
    let mut parts = key.split('.').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            at[part] = value;
            return;
        }
        if !at[part].is_object() {
            at[part] = json!({});
        }
        at = &mut at[part];
    }
}
