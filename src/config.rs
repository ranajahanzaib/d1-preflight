use crate::report::Severity;
use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use serde::Deserialize;
use std::{collections::BTreeMap, path::PathBuf};
#[derive(Parser)]
#[command(
    version,
    about = "Offline migration consequence analysis for Cloudflare D1"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Subcommand)]
pub enum Command {
    /// Apply selected migrations to disposable SQLite and report consequences.
    Check(Box<Options>),
}
#[derive(Args, Debug, Default)]
pub struct Options {
    /// Baseline schema SQL; mutually exclusive with --db
    #[arg(long, conflicts_with = "db")]
    pub schema: Option<PathBuf>,
    /// Existing SQLite database to snapshot read-only (includes committed WAL)
    #[arg(long)]
    pub db: Option<PathBuf>,
    /// Representative seed SQL loaded after --schema
    #[arg(long)]
    pub data: Option<PathBuf>,
    /// Explicit pending migration; repeatable, ordered lexicographically
    #[arg(long)]
    pub migration: Vec<PathBuf>,
    /// Discover pending SQL files using tracking metadata or migrations.after
    #[arg(long)]
    pub migrations_dir: Option<PathBuf>,
    /// SELECT workload SQL file or directory; planned, never executed
    #[arg(long)]
    pub queries: Option<PathBuf>,
    /// Write the detailed versioned JSON report, including execution failures
    #[arg(long)]
    pub json_out: Option<PathBuf>,
    /// Write a bounded Markdown review summary
    #[arg(long)]
    pub markdown_out: Option<PathBuf>,
    /// Emit escaped GitHub annotations and append GITHUB_STEP_SUMMARY
    #[arg(long)]
    pub github: bool,
    /// Fail for findings at or above this severity (default: high)
    #[arg(long, value_enum)]
    pub fail_on: Option<Severity>,
    /// Preflight TOML file (default: ./d1-preflight.toml if present)
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Read basic D1 migration settings from Wrangler TOML; never connects remotely
    #[arg(long)]
    pub wrangler_config: Option<PathBuf>,
    /// Select a D1 binding from --wrangler-config
    #[arg(long)]
    pub binding: Option<String>,
    /// Print execution diagnostics without customer row values
    #[arg(long)]
    pub verbose: bool,
}
#[derive(Deserialize, Default, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub analysis: Analysis,
    pub migrations: Migrations,
    pub queries: Queries,
    pub report: Outputs,
    pub rules: BTreeMap<String, Severity>,
}
#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Analysis {
    pub fail_on: Severity,
    pub check_foreign_keys: bool,
    pub check_integrity: bool,
    pub check_query_plans: bool,
}
impl Default for Analysis {
    fn default() -> Self {
        Self {
            fail_on: Severity::High,
            check_foreign_keys: true,
            check_integrity: true,
            check_query_plans: true,
        }
    }
}
#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Migrations {
    pub directory: Option<PathBuf>,
    pub tracking_table: String,
    pub after: Option<String>,
    pub pattern: Option<String>,
}
impl Default for Migrations {
    fn default() -> Self {
        Self {
            directory: None,
            tracking_table: "d1_migrations".into(),
            after: None,
            pattern: None,
        }
    }
}
#[derive(Deserialize, Default, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Queries {
    pub directory: Option<PathBuf>,
}
#[derive(Deserialize, Default, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Outputs {
    pub json: Option<PathBuf>,
    pub markdown: Option<PathBuf>,
}
pub const RULES: &[&str] = &[
    "DP001", "DP002", "DP003", "SC001", "SC002", "SC003", "SC004", "SC005", "FK001", "FK002",
    "IX001", "IX002", "QP001", "QP002", "SQ001", "OB001", "MG001",
];
impl Config {
    pub fn load(o: &mut Options) -> Result<Self> {
        let path = o.config.clone().or_else(|| {
            PathBuf::from("d1-preflight.toml")
                .exists()
                .then(|| PathBuf::from("d1-preflight.toml"))
        });
        let text = if let Some(p) = &path {
            std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?
        } else {
            String::new()
        };
        let supplied: toml::Value = toml::from_str(&text).context("invalid preflight TOML")?;
        let mut c: Self = toml::from_str(&text).context("invalid preflight configuration")?;
        o.config = path;
        for rule in c.rules.keys() {
            if !RULES.contains(&rule.as_str()) {
                bail!("unknown rule {rule}");
            }
        }
        if let Some(p) = &o.wrangler_config {
            let t: toml::Value = toml::from_str(&std::fs::read_to_string(p)?)?;
            let bindings = t
                .get("d1_databases")
                .and_then(|v| v.as_array())
                .context("Wrangler TOML has no d1_databases")?;
            let selected: Vec<_> = bindings
                .iter()
                .filter(|b| {
                    o.binding.as_ref().is_none_or(|name| {
                        b.get("binding").and_then(|v| v.as_str()) == Some(name.as_str())
                    })
                })
                .collect();
            if selected.len() != 1 {
                bail!("select one D1 binding with --binding");
            }
            let b = selected[0];
            let base = p.parent().unwrap_or(std::path::Path::new("."));
            if c.migrations.directory.is_none() {
                c.migrations.directory = Some(
                    base.join(
                        b.get("migrations_dir")
                            .and_then(|v| v.as_str())
                            .unwrap_or("migrations"),
                    ),
                );
            }
            if let Some(v) = b.get("migrations_table").and_then(|v| v.as_str())
                && supplied
                    .get("migrations")
                    .and_then(|m| m.get("tracking_table"))
                    .is_none()
            {
                c.migrations.tracking_table = v.into();
            }
            if let Some(v) = b.get("migrations_pattern").and_then(|v| v.as_str())
                && c.migrations.pattern.is_none()
            {
                c.migrations.pattern = Some(base.join(v).to_string_lossy().into_owned());
            }
        }
        if o.schema.is_none() == o.db.is_none() {
            bail!("provide exactly one of --schema or --db");
        }
        if o.data.is_some() && o.db.is_some() {
            bail!("--data requires --schema; snapshot data is already present with --db");
        }
        o.migrations_dir = o.migrations_dir.take().or(c.migrations.directory.clone());
        o.queries = o.queries.take().or(c.queries.directory.clone());
        o.json_out = o.json_out.take().or(c.report.json.clone());
        o.markdown_out = o.markdown_out.take().or(c.report.markdown.clone());
        if let Some(s) = o.fail_on {
            c.analysis.fail_on = s;
        }
        Ok(c)
    }
}

/// Reject report destinations aliasing SQL inputs or source database sidecars.
pub fn validate_outputs(o: &Options) -> Result<()> {
    fn resolved(p: &std::path::Path) -> Result<PathBuf> {
        if p.exists() {
            return Ok(p.canonicalize()?);
        }
        let name = p.file_name().context("output must name a file")?;
        let parent = p
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        Ok(resolved(parent)?.join(name))
    }
    let mut inputs: Vec<PathBuf> = o
        .schema
        .iter()
        .chain(o.db.iter())
        .chain(o.data.iter())
        .chain(o.migration.iter())
        .chain(o.config.iter())
        .chain(o.wrangler_config.iter())
        .cloned()
        .collect();
    for path in o.migrations_dir.iter().chain(o.queries.iter()) {
        inputs.extend(crate::engine::files(path)?);
    }
    if let Some(db) = &o.db {
        let canonical = db.canonicalize()?;
        for path in [db, &canonical] {
            for suffix in ["-wal", "-shm", "-journal"] {
                inputs.push(PathBuf::from(format!("{}{suffix}", path.display())));
            }
        }
    }
    let inputs = inputs
        .iter()
        .map(|p| resolved(p))
        .collect::<Result<Vec<_>>>()?;
    let mut outputs = std::collections::BTreeSet::new();
    let summary_path = if o.github {
        std::env::var_os("GITHUB_STEP_SUMMARY").map(PathBuf::from)
    } else {
        None
    };
    for path in o
        .json_out
        .iter()
        .chain(o.markdown_out.iter())
        .chain(summary_path.iter())
    {
        let dest = resolved(path)?;
        if !outputs.insert(dest.clone()) {
            bail!("JSON and Markdown outputs must be distinct");
        }
        for input in &inputs {
            if dest == *input {
                bail!("report output would overwrite input: {}", path.display());
            }
            #[cfg(unix)]
            if let (Ok(a), Ok(b)) = (std::fs::metadata(&dest), std::fs::metadata(input)) {
                use std::os::unix::fs::MetadataExt;
                if a.dev() == b.dev() && a.ino() == b.ino() {
                    bail!("report output aliases input: {}", path.display());
                }
            }
        }
    }
    Ok(())
}
