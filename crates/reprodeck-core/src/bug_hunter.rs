use crate::project_health::{
    HealthCheckResult, HealthCheckStatus, ProjectHealthReport, ProjectProblemRecord,
};
use crate::project_intelligence::{
    CommandConfidence, ProjectCommand, ProjectCommandKind, ProjectProfile,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};

const MAX_PLAN_STEPS: usize = 8;
const MAX_CLUSTER_EXPERIMENTS: usize = 5;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlanStage {
    Diagnostics,
    Tests,
    Build,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RelativeCost {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlannedCheck {
    pub order: usize,
    pub command_id: String,
    pub label: String,
    pub kind: ProjectCommandKind,
    pub executable: String,
    pub args: Vec<String>,
    pub stage: PlanStage,
    pub cost: RelativeCost,
    pub reason_code: String,
    pub source: String,
    pub confidence: CommandConfidence,
    pub after: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanOmission {
    pub command_id: String,
    pub label: String,
    pub reason_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanNotice {
    pub code: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BugHunterPlan {
    pub strategy: String,
    pub project_name: String,
    pub project_fingerprint: String,
    pub steps: Vec<PlannedCheck>,
    pub omitted: Vec<PlanOmission>,
    pub notices: Vec<PlanNotice>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum FailureClass {
    Compilation,
    TypeSystem,
    Test,
    Lint,
    Build,
    Timeout,
    Execution,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InvestigationExperiment {
    pub order: usize,
    pub kind: String,
    pub title: String,
    pub purpose: String,
    pub command_id: Option<String>,
    pub requires_evidence: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FailureCluster {
    pub id: String,
    pub class: FailureClass,
    pub signature: String,
    pub title: String,
    pub summary: String,
    pub check_ids: Vec<String>,
    pub command_ids: Vec<String>,
    pub evidence_ids: Vec<String>,
    pub related_problem_ids: Vec<String>,
    pub investigation_query: String,
    pub experiments: Vec<InvestigationExperiment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionBlocker {
    pub command_id: String,
    pub label: String,
    pub summary: String,
    pub evidence_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BugHunterAnalysis {
    pub health_run_id: String,
    pub clusters: Vec<FailureCluster>,
    pub blockers: Vec<ExecutionBlocker>,
    pub failed_checks: usize,
    pub clustered_failures: usize,
}

fn stage_for(kind: ProjectCommandKind) -> Option<PlanStage> {
    match kind {
        ProjectCommandKind::Check | ProjectCommandKind::Typecheck | ProjectCommandKind::Lint => {
            Some(PlanStage::Diagnostics)
        }
        ProjectCommandKind::Test => Some(PlanStage::Tests),
        ProjectCommandKind::Build => Some(PlanStage::Build),
        ProjectCommandKind::Dev | ProjectCommandKind::Other => None,
    }
}

fn cost_for(kind: ProjectCommandKind) -> RelativeCost {
    match kind {
        ProjectCommandKind::Check | ProjectCommandKind::Typecheck | ProjectCommandKind::Lint => {
            RelativeCost::Low
        }
        ProjectCommandKind::Test => RelativeCost::Medium,
        ProjectCommandKind::Build => RelativeCost::High,
        ProjectCommandKind::Dev | ProjectCommandKind::Other => RelativeCost::High,
    }
}

fn reason_code(kind: ProjectCommandKind) -> &'static str {
    match kind {
        ProjectCommandKind::Check => "compiler-diagnostics",
        ProjectCommandKind::Typecheck => "type-diagnostics",
        ProjectCommandKind::Lint => "lint-diagnostics",
        ProjectCommandKind::Test => "behavioral-tests",
        ProjectCommandKind::Build => "release-shape",
        ProjectCommandKind::Dev | ProjectCommandKind::Other => "manual-only",
    }
}

fn priority(command: &ProjectCommand) -> (u8, u8, String, String) {
    let stage_rank = match command.kind {
        ProjectCommandKind::Check => 10,
        ProjectCommandKind::Typecheck => 20,
        ProjectCommandKind::Lint => 30,
        ProjectCommandKind::Test => 40,
        ProjectCommandKind::Build => 50,
        ProjectCommandKind::Dev => 90,
        ProjectCommandKind::Other => 100,
    };
    let confidence_rank = match command.confidence {
        CommandConfidence::Declared => 0,
        CommandConfidence::Conventional => 1,
    };
    (
        stage_rank,
        confidence_rank,
        command.label.to_ascii_lowercase(),
        command.id.clone(),
    )
}

fn command_identity(command: &ProjectCommand) -> String {
    let mut key = command.executable.to_ascii_lowercase();
    for arg in &command.args {
        key.push('\0');
        key.push_str(arg);
    }
    key
}

pub fn build_plan(profile: &ProjectProfile) -> BugHunterPlan {
    let mut candidates = profile.commands.clone();
    candidates.sort_by_key(priority);

    let mut seen = HashSet::new();
    let mut steps: Vec<PlannedCheck> = Vec::new();
    let mut omitted = Vec::new();

    for command in candidates {
        let Some(stage) = stage_for(command.kind) else {
            omitted.push(PlanOmission {
                command_id: command.id,
                label: command.label,
                reason_code: "manual-only".into(),
            });
            continue;
        };

        let identity = command_identity(&command);
        if !seen.insert(identity) {
            omitted.push(PlanOmission {
                command_id: command.id,
                label: command.label,
                reason_code: "duplicate-command".into(),
            });
            continue;
        }

        if steps.len() >= MAX_PLAN_STEPS {
            omitted.push(PlanOmission {
                command_id: command.id,
                label: command.label,
                reason_code: "plan-budget".into(),
            });
            continue;
        }

        let after = match stage {
            PlanStage::Diagnostics => Vec::new(),
            PlanStage::Tests => steps
                .iter()
                .filter(|step| step.stage == PlanStage::Diagnostics)
                .map(|step| step.command_id.clone())
                .collect(),
            PlanStage::Build => steps
                .iter()
                .filter(|step| step.stage != PlanStage::Build)
                .map(|step| step.command_id.clone())
                .collect(),
        };

        steps.push(PlannedCheck {
            order: steps.len() + 1,
            command_id: command.id,
            label: command.label,
            kind: command.kind,
            executable: command.executable,
            args: command.args,
            stage,
            cost: cost_for(command.kind),
            reason_code: reason_code(command.kind).into(),
            source: command.source,
            confidence: command.confidence,
            after,
        });
    }

    let mut notices = Vec::new();
    if profile.git.is_none() {
        notices.push(PlanNotice {
            code: "git-required".into(),
            detail: "Automatic execution is unavailable until the project is inside a Git repository with a committed HEAD.".into(),
        });
    } else if profile.git.as_ref().is_some_and(|git| git.is_dirty) {
        notices.push(PlanNotice {
            code: "dirty-head-only".into(),
            detail: "The plan will execute the committed HEAD in a disposable worktree; current local edits are intentionally excluded.".into(),
        });
    }
    if profile.stats.scan_truncated {
        notices.push(PlanNotice {
            code: "discovery-truncated".into(),
            detail: "Project discovery reached its bounded scan budget; the plan uses only commands that were actually detected.".into(),
        });
    }
    if steps.is_empty() {
        notices.push(PlanNotice {
            code: "no-deterministic-checks".into(),
            detail: "No deterministic build, test, lint, typecheck or compiler-check command was detected.".into(),
        });
    }

    BugHunterPlan {
        strategy: "diagnostics-first-v1".into(),
        project_name: profile.name.clone(),
        project_fingerprint: profile.fingerprint.clone(),
        steps,
        omitted,
        notices,
    }
}

fn meaningful_lines(check: &HealthCheckResult) -> Vec<String> {
    check
        .stderr_preview
        .lines()
        .chain(check.stdout_preview.lines())
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(120)
        .map(str::to_owned)
        .collect()
}

fn diagnostic_score(line: &str) -> u8 {
    let lower = line.to_ascii_lowercase();
    let mut score = 0;
    if lower.contains("error") {
        score += 5;
    }
    if lower.contains("failed") || lower.contains("failure") || lower.contains("panic") {
        score += 4;
    }
    if lower.contains("exception") || lower.contains("assert") {
        score += 3;
    }
    if lower.contains("not found") || lower.contains("cannot find") || lower.contains("unresolved")
    {
        score += 3;
    }
    if lower.contains("ts") && lower.chars().any(|ch| ch.is_ascii_digit()) {
        score += 2;
    }
    if lower.contains(" --> ") || lower.contains(" at ") {
        score += 1;
    }
    score
}

fn primary_diagnostic(check: &HealthCheckResult) -> String {
    let lines = meaningful_lines(check);
    lines
        .iter()
        .enumerate()
        .max_by(|(left_idx, left), (right_idx, right)| {
            diagnostic_score(left)
                .cmp(&diagnostic_score(right))
                .then_with(|| right_idx.cmp(left_idx))
        })
        .map(|(_, line)| line.clone())
        .or_else(|| (!check.summary.trim().is_empty()).then(|| check.summary.trim().to_owned()))
        .unwrap_or_else(|| format!("{} failed", check.label))
}

fn strip_root(value: &str, root: &str) -> String {
    if root.is_empty() {
        return value.to_owned();
    }
    let plain = root.strip_prefix(r"\\?\").unwrap_or(root);
    let variants = [
        root.to_owned(),
        root.replace('\\', "/"),
        root.replace('/', "\\"),
        plain.to_owned(),
        plain.replace('\\', "/"),
        plain.replace('/', "\\"),
    ];
    variants
        .into_iter()
        .filter(|item| !item.is_empty())
        .fold(value.to_owned(), |current, item| {
            current.replace(&item, "[PROJECT_ROOT]")
        })
}

fn normalize_signature(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    let mut out = String::with_capacity(lower.len().min(220));
    let mut digit_run = false;
    let mut whitespace = false;
    for ch in lower.chars().take(400) {
        if ch.is_ascii_digit() {
            if !digit_run {
                out.push('#');
                digit_run = true;
            }
            whitespace = false;
            continue;
        }
        digit_run = false;
        if ch.is_whitespace() {
            if !whitespace {
                out.push(' ');
                whitespace = true;
            }
            continue;
        }
        whitespace = false;
        out.push(ch);
        if out.len() >= 220 {
            break;
        }
    }
    out.trim().to_owned()
}

fn diagnostic_code(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    for index in 0..bytes.len() {
        if bytes[index].eq_ignore_ascii_case(&b't')
            && index + 3 < bytes.len()
            && bytes[index + 1].eq_ignore_ascii_case(&b's')
        {
            let mut end = index + 2;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end >= index + 5 {
                return Some(value[index..end].to_ascii_lowercase());
            }
        }
        if bytes[index].eq_ignore_ascii_case(&b'e') && index + 4 < bytes.len() {
            let mut end = index + 1;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end >= index + 5 {
                return Some(value[index..end].to_ascii_lowercase());
            }
        }
    }
    None
}

fn failure_class(check: &HealthCheckResult, diagnostic: &str) -> FailureClass {
    if check.status == HealthCheckStatus::TimedOut {
        return FailureClass::Timeout;
    }
    if check.status == HealthCheckStatus::Error {
        return FailureClass::Execution;
    }
    let lower = diagnostic.to_ascii_lowercase();
    if let Some(code) = diagnostic_code(diagnostic) {
        if code.starts_with("ts") {
            return FailureClass::TypeSystem;
        }
        return FailureClass::Compilation;
    }
    if lower.contains("cannot find") || lower.contains("unresolved import") {
        if check.kind == ProjectCommandKind::Typecheck {
            return FailureClass::TypeSystem;
        }
        return FailureClass::Compilation;
    }
    match check.kind {
        ProjectCommandKind::Typecheck => FailureClass::TypeSystem,
        ProjectCommandKind::Check => FailureClass::Compilation,
        ProjectCommandKind::Test => FailureClass::Test,
        ProjectCommandKind::Lint => FailureClass::Lint,
        ProjectCommandKind::Build => FailureClass::Build,
        ProjectCommandKind::Dev | ProjectCommandKind::Other => FailureClass::Unknown,
    }
}

fn failure_signature(diagnostic: &str) -> String {
    let normalized = normalize_signature(diagnostic);
    match diagnostic_code(diagnostic) {
        Some(code) => format!("{code} · {normalized}"),
        None => normalized,
    }
}

fn cluster_key(class: FailureClass, diagnostic: &str) -> String {
    format!("{class:?}:{}", failure_signature(diagnostic))
}

fn cluster_id(key: &str) -> String {
    let digest = Sha256::digest(key.as_bytes());
    format!("cluster:{}", &hex::encode(digest)[..20])
}

fn class_title(class: FailureClass) -> &'static str {
    match class {
        FailureClass::Compilation => "Compiler failure",
        FailureClass::TypeSystem => "Type-check failure",
        FailureClass::Test => "Test failure",
        FailureClass::Lint => "Lint failure",
        FailureClass::Build => "Build failure",
        FailureClass::Timeout => "Check timed out",
        FailureClass::Execution => "Check execution error",
        FailureClass::Unknown => "Project check failure",
    }
}

fn experiments_for(
    class: FailureClass,
    command_ids: &[String],
    signature: &str,
) -> Vec<InvestigationExperiment> {
    let primary = command_ids.first().cloned();
    let mut items = vec![
        InvestigationExperiment {
            order: 1,
            kind: "reproduce".into(),
            title: "Reproduce the smallest deterministic failure".into(),
            purpose: "Confirm the same failure still occurs before forming a root-cause claim.".into(),
            command_id: primary.clone(),
            requires_evidence: true,
        },
        InvestigationExperiment {
            order: 2,
            kind: "context".into(),
            title: "Compile focused repository context".into(),
            purpose: format!("Retrieve code around the diagnostic signature without sending the whole repository: {signature}"),
            command_id: None,
            requires_evidence: true,
        },
    ];

    let (title, purpose) = match class {
        FailureClass::Compilation | FailureClass::TypeSystem => (
            "Trace the first compiler diagnostic",
            "Inspect the first concrete diagnostic, its referenced symbol/import/type and the smallest dependency chain that can explain it.",
        ),
        FailureClass::Test => (
            "Trace the first failing assertion or test case",
            "Separate the observed behavior from later cascading test output and identify the state transition that first diverges.",
        ),
        FailureClass::Lint => (
            "Inspect the exact lint rule and source location",
            "Determine whether the lint is a correctness signal, configuration mismatch or style-only finding before proposing code changes.",
        ),
        FailureClass::Build => (
            "Locate the first build-stage failure",
            "Distinguish source errors from bundler, asset, configuration and packaging failures before changing application logic.",
        ),
        FailureClass::Timeout => (
            "Determine where progress stops",
            "Capture the last deterministic output and distinguish a slow check from a deadlock, wait-on-service or runaway child process.",
        ),
        FailureClass::Execution | FailureClass::Unknown => (
            "Separate project failure from runner failure",
            "Verify executable availability, arguments and project prerequisites before treating the result as a source-code bug.",
        ),
    };
    items.push(InvestigationExperiment {
        order: 3,
        kind: "trace".into(),
        title: title.into(),
        purpose: purpose.into(),
        command_id: primary.clone(),
        requires_evidence: true,
    });
    items.push(InvestigationExperiment {
        order: 4,
        kind: "hypothesis".into(),
        title: "Test one falsifiable root-cause hypothesis".into(),
        purpose: "Change one variable in the isolated workspace, record what would disprove the hypothesis, then rerun the same criterion.".into(),
        command_id: primary.clone(),
        requires_evidence: true,
    });
    items.push(InvestigationExperiment {
        order: 5,
        kind: "regression".into(),
        title: "Run broader regression checks only after the primary failure is fixed".into(),
        purpose: "A passing narrow reproduction is necessary but not sufficient; the surrounding deterministic checks must remain green.".into(),
        command_id: None,
        requires_evidence: true,
    });
    items.truncate(MAX_CLUSTER_EXPERIMENTS);
    items
}

fn investigation_query(cluster: &FailureCluster) -> String {
    format!(
        "Investigate this reproduced {:?} failure. Diagnostic signature: {}. Commands: {}. Evidence: {}. Separate observations from hypotheses, identify the earliest causal failure, state what would falsify the leading hypothesis, and propose the smallest deterministic experiment before any fix.",
        cluster.class,
        cluster.signature,
        cluster.command_ids.join(", "),
        cluster.evidence_ids.join(", ")
    )
}

pub fn analyze_failures(
    report: &ProjectHealthReport,
    problems: &[ProjectProblemRecord],
) -> BugHunterAnalysis {
    #[derive(Default)]
    struct Accumulator {
        class: Option<FailureClass>,
        signature: String,
        diagnostics: Vec<String>,
        check_ids: Vec<String>,
        command_ids: Vec<String>,
        evidence_ids: Vec<String>,
    }

    let mut groups: BTreeMap<String, Accumulator> = BTreeMap::new();
    let mut blockers = Vec::new();
    let mut failed_checks = 0;

    for check in &report.checks {
        match check.status {
            HealthCheckStatus::Blocked => {
                blockers.push(ExecutionBlocker {
                    command_id: check.command_id.clone(),
                    label: check.label.clone(),
                    summary: check.summary.clone(),
                    evidence_id: check.evidence_id.clone(),
                });
            }
            HealthCheckStatus::Failed | HealthCheckStatus::TimedOut | HealthCheckStatus::Error => {
                failed_checks += 1;
                let diagnostic = strip_root(&primary_diagnostic(check), &report.root_path);
                let class = failure_class(check, &diagnostic);
                let key = cluster_key(class, &diagnostic);
                let item = groups.entry(key).or_default();
                item.class = Some(class);
                item.signature = failure_signature(&diagnostic);
                if !item.diagnostics.contains(&diagnostic) {
                    item.diagnostics.push(diagnostic);
                }
                if !item.check_ids.contains(&check.id) {
                    item.check_ids.push(check.id.clone());
                }
                if !item.command_ids.contains(&check.command_id) {
                    item.command_ids.push(check.command_id.clone());
                }
                if !item.evidence_ids.contains(&check.evidence_id) {
                    item.evidence_ids.push(check.evidence_id.clone());
                }
            }
            HealthCheckStatus::Passed => {}
        }
    }

    let problem_by_command: HashMap<&str, Vec<&ProjectProblemRecord>> = problems
        .iter()
        .filter(|problem| problem.active)
        .fold(HashMap::new(), |mut map, problem| {
            map.entry(problem.command_id.as_str())
                .or_default()
                .push(problem);
            map
        });

    let mut clusters = Vec::new();
    for (key, group) in groups {
        let class = group.class.unwrap_or(FailureClass::Unknown);
        let mut related_problem_ids = Vec::new();
        for command in &group.command_ids {
            if let Some(items) = problem_by_command.get(command.as_str()) {
                for problem in items {
                    if !related_problem_ids.contains(&problem.id) {
                        related_problem_ids.push(problem.id.clone());
                    }
                }
            }
        }
        let summary = group
            .diagnostics
            .first()
            .cloned()
            .unwrap_or_else(|| "The check failed without a diagnostic line.".into());
        let mut cluster = FailureCluster {
            id: cluster_id(&key),
            class,
            signature: group.signature,
            title: class_title(class).into(),
            summary,
            check_ids: group.check_ids,
            command_ids: group.command_ids,
            evidence_ids: group.evidence_ids,
            related_problem_ids,
            investigation_query: String::new(),
            experiments: Vec::new(),
        };
        cluster.experiments =
            experiments_for(cluster.class, &cluster.command_ids, &cluster.signature);
        cluster.investigation_query = investigation_query(&cluster);
        clusters.push(cluster);
    }

    clusters.sort_by(|left, right| {
        left.class
            .cmp(&right.class)
            .then_with(|| right.evidence_ids.len().cmp(&left.evidence_ids.len()))
            .then_with(|| left.signature.cmp(&right.signature))
    });
    blockers.sort_by(|left, right| left.command_id.cmp(&right.command_id));

    BugHunterAnalysis {
        health_run_id: report.id.clone(),
        clustered_failures: clusters.len(),
        failed_checks,
        clusters,
        blockers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_health::{HealthRunStatus, ProjectHealthReport};
    use crate::project_intelligence::{ProjectGitState, ProjectStats};

    fn command(
        id: &str,
        kind: ProjectCommandKind,
        executable: &str,
        args: &[&str],
        confidence: CommandConfidence,
    ) -> ProjectCommand {
        ProjectCommand {
            id: id.into(),
            label: id.into(),
            kind,
            executable: executable.into(),
            args: args.iter().map(|value| (*value).into()).collect(),
            source: "fixture".into(),
            confidence,
        }
    }

    fn profile(commands: Vec<ProjectCommand>) -> ProjectProfile {
        ProjectProfile {
            schema_version: 1,
            fingerprint: "fingerprint".into(),
            root_path: "C:/repo".into(),
            name: "fixture".into(),
            version: None,
            description: None,
            analyzed_at: 1,
            git: Some(ProjectGitState {
                root_path: "C:/repo".into(),
                branch: "main".into(),
                head_commit: Some("abc".into()),
                is_dirty: false,
                changed_files: Vec::new(),
            }),
            languages: Vec::new(),
            technologies: Vec::new(),
            commands,
            entrypoints: Vec::new(),
            test_paths: Vec::new(),
            documentation: Vec::new(),
            ci_files: Vec::new(),
            signals: Vec::new(),
            stats: ProjectStats::default(),
        }
    }

    fn failed_check(
        id: &str,
        command_id: &str,
        kind: ProjectCommandKind,
        stderr: &str,
    ) -> HealthCheckResult {
        HealthCheckResult {
            id: id.into(),
            command_id: command_id.into(),
            label: command_id.into(),
            kind,
            executable: "tool".into(),
            args: Vec::new(),
            status: HealthCheckStatus::Failed,
            exit_code: Some(1),
            duration_ms: 12,
            stdout_preview: String::new(),
            stderr_preview: stderr.into(),
            stdout_truncated: false,
            stderr_truncated: false,
            evidence_id: format!("health:run:{id}"),
            summary: stderr.into(),
        }
    }

    fn report(checks: Vec<HealthCheckResult>) -> ProjectHealthReport {
        ProjectHealthReport {
            id: "run".into(),
            root_path: "C:/repo".into(),
            project_name: "fixture".into(),
            base_commit: "abc".into(),
            started_at: 1,
            finished_at: 2,
            status: HealthRunStatus::ProblemsFound,
            original_unchanged: true,
            source_had_local_changes: false,
            checks,
            problems: Vec::new(),
        }
    }

    #[test]
    fn plan_is_diagnostics_first_and_declared_before_conventional() {
        let plan = build_plan(&profile(vec![
            command(
                "build",
                ProjectCommandKind::Build,
                "npm",
                &["run", "build"],
                CommandConfidence::Declared,
            ),
            command(
                "test",
                ProjectCommandKind::Test,
                "npm",
                &["test"],
                CommandConfidence::Declared,
            ),
            command(
                "typecheck-conventional",
                ProjectCommandKind::Typecheck,
                "npx",
                &["tsc"],
                CommandConfidence::Conventional,
            ),
            command(
                "typecheck-declared",
                ProjectCommandKind::Typecheck,
                "npm",
                &["run", "typecheck"],
                CommandConfidence::Declared,
            ),
            command(
                "check",
                ProjectCommandKind::Check,
                "cargo",
                &["check"],
                CommandConfidence::Declared,
            ),
        ]));
        let ids = plan
            .steps
            .iter()
            .map(|step| step.command_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "check",
                "typecheck-declared",
                "typecheck-conventional",
                "test",
                "build"
            ]
        );
        assert_eq!(plan.steps[0].stage, PlanStage::Diagnostics);
        assert_eq!(plan.steps.last().unwrap().stage, PlanStage::Build);
    }

    #[test]
    fn plan_deduplicates_identical_commands_and_omits_dev() {
        let plan = build_plan(&profile(vec![
            command(
                "test-a",
                ProjectCommandKind::Test,
                "cargo",
                &["test"],
                CommandConfidence::Declared,
            ),
            command(
                "test-b",
                ProjectCommandKind::Test,
                "cargo",
                &["test"],
                CommandConfidence::Conventional,
            ),
            command(
                "dev",
                ProjectCommandKind::Dev,
                "npm",
                &["run", "dev"],
                CommandConfidence::Declared,
            ),
        ]));
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].command_id, "test-a");
        assert!(plan
            .omitted
            .iter()
            .any(|item| item.reason_code == "duplicate-command"));
        assert!(plan
            .omitted
            .iter()
            .any(|item| item.reason_code == "manual-only"));
    }

    #[test]
    fn failures_with_same_compiler_code_are_clustered() {
        let analysis = analyze_failures(
            &report(vec![
                failed_check(
                    "one",
                    "check",
                    ProjectCommandKind::Check,
                    "error[E0308]: mismatched types at C:/repo/src/a.rs:10:2",
                ),
                failed_check(
                    "two",
                    "build",
                    ProjectCommandKind::Build,
                    "error[E0308]: mismatched types at C:/repo/src/a.rs:11:7",
                ),
            ]),
            &[],
        );
        assert_eq!(analysis.failed_checks, 2);
        assert_eq!(analysis.clusters.len(), 1);
        assert!(analysis.clusters[0].signature.starts_with("e0308 · "));
        assert_eq!(analysis.clusters[0].evidence_ids.len(), 2);
        assert!(!analysis.clusters[0].investigation_query.contains("C:/repo"));
    }

    #[test]
    fn same_compiler_code_in_different_files_is_not_assumed_to_share_a_root_cause() {
        let analysis = analyze_failures(
            &report(vec![
                failed_check(
                    "one",
                    "check-a",
                    ProjectCommandKind::Check,
                    "error[E0308]: mismatched types at src/a.rs:10:2",
                ),
                failed_check(
                    "two",
                    "check-b",
                    ProjectCommandKind::Check,
                    "error[E0308]: mismatched types at src/b.rs:11:7",
                ),
            ]),
            &[],
        );
        assert_eq!(analysis.clusters.len(), 2);
    }

    #[test]
    fn extended_windows_project_root_is_removed_from_cluster_text() {
        let mut report = report(vec![failed_check(
            "one",
            "check",
            ProjectCommandKind::Check,
            r"error: failed at C:\Users\private\repo\src\main.rs:10:2",
        )]);
        report.root_path = r"\\?\C:\Users\private\repo".into();
        let analysis = analyze_failures(&report, &[]);
        let cluster = &analysis.clusters[0];
        assert!(!cluster.summary.contains(r"C:\Users\private\repo"));
        assert!(!cluster
            .investigation_query
            .contains(r"C:\Users\private\repo"));
        assert!(cluster.summary.contains("[PROJECT_ROOT]"));
    }

    #[test]
    fn blocked_checks_are_not_reported_as_bug_clusters() {
        let mut blocked = failed_check(
            "blocked",
            "test",
            ProjectCommandKind::Test,
            "dependencies missing",
        );
        blocked.status = HealthCheckStatus::Blocked;
        let analysis = analyze_failures(&report(vec![blocked]), &[]);
        assert_eq!(analysis.failed_checks, 0);
        assert!(analysis.clusters.is_empty());
        assert_eq!(analysis.blockers.len(), 1);
    }

    #[test]
    fn different_failure_classes_do_not_collapse_together() {
        let analysis = analyze_failures(
            &report(vec![
                failed_check(
                    "test",
                    "test",
                    ProjectCommandKind::Test,
                    "AssertionError: expected 2 got 3",
                ),
                failed_check(
                    "lint",
                    "lint",
                    ProjectCommandKind::Lint,
                    "error: unused variable count 2",
                ),
            ]),
            &[],
        );
        assert_eq!(analysis.clusters.len(), 2);
        assert_ne!(analysis.clusters[0].class, analysis.clusters[1].class);
    }
}
