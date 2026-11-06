use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}
impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", format!("{self:?}").to_lowercase())
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub rule_id: String,
    pub severity: Severity,
    pub confidence: String,
    pub title: String,
    pub description: String,
    pub object: String,
    pub location: Option<String>,
    pub evidence: serde_json::Value,
    pub recommendation: String,
}
#[derive(Debug, Serialize)]
pub struct Migration {
    pub file: String,
    pub order: usize,
    pub status: String,
    pub error: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct PlanStep {
    pub id: i64,
    pub parent: i64,
    pub detail: String,
}
#[derive(Debug, Serialize)]
pub struct Query {
    pub file: String,
    pub before: Vec<String>,
    pub after: Vec<String>,
    pub before_steps: Vec<PlanStep>,
    pub after_steps: Vec<PlanStep>,
    pub baseline_error: bool,
    pub candidate_error: bool,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: String,
    pub tool_version: String,
    pub sqlite_version: String,
    pub status: String,
    pub baseline_source: String,
    pub migrations: Vec<Migration>,
    pub schema_changes: Vec<String>,
    pub findings: Vec<Finding>,
    pub summary: BTreeMap<String, usize>,
    pub queries: Vec<Query>,
    pub checks: BTreeMap<String, String>,
    pub limitations: Vec<String>,
}
impl Report {
    pub fn new(source: String) -> Self {
        Self { schema_version: "1.0".into(), tool_version: env!("CARGO_PKG_VERSION").into(), sqlite_version: rusqlite::version().into(), status: "completed".into(), baseline_source: source, migrations: vec![], schema_changes: vec![], findings: vec![], summary: BTreeMap::new(), queries: vec![], checks: BTreeMap::new(), limitations: vec!["Local SQLite behavior is verified; D1 runtime compatibility is unverified. Passing is not deployment approval.".into(), "Query plans are observations, not latency measurements. Workload SQL is never executed; parameters are NULL for planning.".into(), "Only supplied rows are tested. Application dependencies, CHECK/collation expressions and ambiguous renames require review.".into()] }
    }
    #[allow(clippy::too_many_arguments)] // Each argument is a required finding field.
    pub fn finding(
        &mut self,
        rule: &str,
        severity: Severity,
        title: &str,
        object: &str,
        location: Option<&str>,
        evidence: serde_json::Value,
        recommendation: &str,
    ) {
        self.findings.push(Finding {
            rule_id: rule.into(),
            severity,
            confidence: if ["SQ001", "QP001"].contains(&rule) {
                "medium"
            } else {
                "high"
            }
            .into(),
            title: title.into(),
            description: rule_description(rule).into(),
            object: object.into(),
            location: location.map(str::to_owned),
            evidence,
            recommendation: recommendation.into(),
        });
    }
    pub fn finish(&mut self, overrides: &BTreeMap<String, Severity>) {
        for f in &mut self.findings {
            if let Some(s) = overrides.get(&f.rule_id) {
                f.severity = *s;
            }
        }
        self.findings.sort_by(|a, b| {
            b.severity
                .cmp(&a.severity)
                .then(a.rule_id.cmp(&b.rule_id))
                .then(a.object.cmp(&b.object))
                .then(a.location.cmp(&b.location))
        });
        for s in [
            Severity::Critical,
            Severity::High,
            Severity::Medium,
            Severity::Low,
        ] {
            self.summary.insert(
                s.to_string(),
                self.findings.iter().filter(|f| f.severity == s).count(),
            );
        }
    }
    pub fn failed(&self, threshold: Severity) -> bool {
        self.status != "completed" || self.findings.iter().any(|f| f.severity >= threshold)
    }
    pub fn markdown(&self) -> String {
        let mut s = format!(
            "# D1 Preflight\n\nStatus: **{}** · SQLite {}\n\n{} migrations · {} workload queries\n\n",
            self.status,
            self.sqlite_version,
            self.migrations.len(),
            self.queries.len()
        );
        for f in self.findings.iter().take(100) {
            s.push_str(&format!(
                "- **{} {}** {} (`{}`): {}\n",
                f.severity,
                f.rule_id,
                clean(&f.title),
                clean(&f.object),
                clean(&f.recommendation)
            ));
        }
        if self.findings.is_empty() {
            s.push_str("No issues detected within the supplied coverage.\n");
        }
        s.push_str("\n## Schema changes\n\n");
        for c in &self.schema_changes {
            s.push_str(&format!("- {}\n", clean(c)));
        }
        s.push_str("\n## Coverage and limitations\n\n");
        for (k, v) in &self.checks {
            s.push_str(&format!("- {}: {}\n", clean(k), clean(v)));
        }
        for l in &self.limitations {
            s.push_str(&format!("- {l}\n"));
        }
        if s.len() > 60_000 {
            let mut end = 60_000;
            while !s.is_char_boundary(end) {
                end -= 1;
            }
            s.truncate(end);
            s.push_str("\n\nSummary truncated; consult the JSON report.\n");
        }
        s
    }
    pub fn terminal(&self) -> String {
        let mut s = format!(
            "D1 PREFLIGHT\nSQLite {} | {} | {} migrations\n",
            self.sqlite_version,
            self.status,
            self.migrations.len()
        );
        for c in &self.schema_changes {
            s.push_str(&format!("  {c}\n"));
        }
        for f in &self.findings {
            s.push_str(&format!(
                "\n{} {} — {} [{}]\n  Evidence: {}\n  {}\n",
                f.severity.to_string().to_uppercase(),
                f.rule_id,
                f.title,
                f.object,
                f.evidence,
                f.recommendation
            ));
        }
        if self.findings.is_empty() {
            s.push_str("No issues detected within supplied coverage.\n");
        }
        s.push_str("\nCHECKS\n");
        for (check, status) in &self.checks {
            s.push_str(&format!("  {check}: {status}\n"));
        }
        s.push_str(&format!(
            "\nRESULT\n  {} critical · {} high · {} medium · {} low\n",
            self.summary.get("critical").unwrap_or(&0),
            self.summary.get("high").unwrap_or(&0),
            self.summary.get("medium").unwrap_or(&0),
            self.summary.get("low").unwrap_or(&0)
        ));
        s.push_str("Local SQLite analysis; production D1 behavior remains unverified.\n");
        s
    }
}
fn clean(s: &str) -> String {
    s.replace(['\n', '\r', '`', '<', '>'], " ")
}
pub fn escape_command(s: &str) -> String {
    s.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
        .replace(':', "%3A")
        .replace(',', "%2C")
}
pub fn github(report: &Report) -> anyhow::Result<()> {
    for f in &report.findings {
        let kind = if f.severity >= Severity::High {
            "error"
        } else {
            "warning"
        };
        let file = f
            .location
            .as_deref()
            .map(|v| format!("file={},", escape_command(v)))
            .unwrap_or_default();
        println!(
            "::{kind} {file}title={}::{}",
            escape_command(&format!("{} {}", f.rule_id, f.title)),
            escape_command(&f.recommendation)
        );
    }
    if let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().append(true).open(path)?;
        file.write_all(report.markdown().as_bytes())?;
    }
    Ok(())
}

fn rule_description(rule: &str) -> &'static str {
    match rule {
        "DP001" => {
            "The final schema no longer contains this table. Application references can fail and supplied records are no longer accessible there."
        }
        "DP002" => {
            "The final table no longer exposes this column. Existing application reads and writes can fail, even when the baseline has no rows."
        }
        "DP003" => {
            "Supplied record counts decreased or primary key identities disappeared. This is evidence of removal or re-keying, not proof of lost values when identity changes were intentional."
        }
        "SC001" => {
            "Column or table enforcement/storage metadata changed. Previously accepted values or application assumptions may no longer hold."
        }
        "SC002" => {
            "The candidate requires a value where the baseline did not. Untested rows or writers may violate this new requirement."
        }
        "SC003" => {
            "The ordered primary key changed. Record identity and referencing relationships require review."
        }
        "SC004" => {
            "The default expression changed. Inserts omitting this column may now produce different values."
        }
        "SC005" => {
            "A baseline unique guarantee has no confirmed equivalent candidate index. Future writes may accept duplicates."
        }
        "FK001" => {
            "SQLite observed orphan relationships or could not validate foreign key metadata. Referential integrity has not passed."
        }
        "FK002" => {
            "An existing foreign key definition changed or disappeared. Relationship enforcement or cascading behavior may differ."
        }
        "IX001" => {
            "An existing index disappeared or changed. Workloads may lose an access path; latency impact has not been measured."
        }
        "IX002" => {
            "Local EXPLAIN QUERY PLAN shows a less selective lookup, new temporary sorting, or new automatic indexing. This indicates potential additional work, not a measured production slowdown."
        }
        "QP001" => {
            "The candidate workload plan contains a full scan. Cost depends on table size and purpose; a scan may be appropriate."
        }
        "QP002" => {
            "A supplied SELECT prepared against the baseline but fails against the candidate. Its schema dependencies are no longer satisfied locally."
        }
        "SQ001" => {
            "A parsed top-level mutation has no WHERE predicate. Its scope deserves review; the tool cannot determine whether the operation is intentional."
        }
        "OB001" => {
            "A view or trigger disappeared or its SQL tokens changed. Consumers or database side effects may differ."
        }
        "MG001" => {
            "Migration execution, commit, supported execution policy, or database integrity validation failed. The rejected file is not considered successfully applied."
        }
        _ => "Review the observed evidence and application impact.",
    }
}
