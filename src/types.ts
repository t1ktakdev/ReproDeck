export type BridgeError = { code?: string; message?: string };

export type Session = {
  created_seq: number;
  id: string;
  repo_id: string | null;
  created_at: number;
  updated_at: number | null;
  state: string;
  meta: string | null;
};

export type SessionMeta = { title: string; expected: string; actual: string; notes: string };

export type RepositoryInfo = {
  id: string | null;
  path: string;
  head_commit: string;
  branch: string;
  is_dirty: boolean;
  changed_files: string[];
};

export type StoredRepository = {
  id: string;
  path: string;
  stored_head_commit: string | null;
  current: RepositoryInfo | null;
  accessible: boolean;
};

export type EnvironmentSnapshot = {
  id: string;
  session_id: string;
  captured_at: number;
  os: string;
  arch: string;
  git_version: string | null;
  runtimes: Record<string, string>;
};

export type ReproductionStep = {
  id: string;
  session_id: string;
  ordering: number;
  executable: string;
  args: string[];
  expected_exit_code: number;
  active_cycle: number;
  created_at: number;
};

export type ReproductionRun = {
  id: string;
  step_id: string;
  phase: "Before" | "After";
  action_id: string;
  receipt_id: string | null;
  exit_code: number | null;
  status: string;
  cycle: number;
  created_at: number;
};

export type ShadowWorkspace = {
  session_id: string;
  repo_id: string;
  repo_path: string;
  base_commit: string;
  branch: string;
  worktree_path: string;
  original_branch: string;
  dirty: boolean;
};

export type ShadowDiff = { patch: string; files: string[] };

export type ActionRecord = {
  created_seq: number;
  id: string;
  session_id: string;
  parent_id: string | null;
  kind: string;
  meta: string | null;
  state: string;
  created_at: number;
};

export type ExecutionRecord = {
  created_seq: number;
  id: string;
  action_id: string;
  status: string;
  started_at: number;
  finished_at: number | null;
  duration_ms: number | null;
};

export type ReceiptRecord = {
  created_seq: number;
  id: string;
  execution_id: string;
  summary: string | null;
  stdout_preview: string | null;
  stderr_preview: string | null;
  stdout_truncated: boolean;
  stderr_truncated: boolean;
  created_at: number;
};

export type ArtifactRecord = {
  created_seq: number;
  id: string;
  receipt_id: string;
  store_key: string;
  checksum: string;
  size: number;
  media_type: string | null;
  created_at: number;
};

export type EvidenceItem = {
  created_seq: number;
  id: string;
  session_id: string;
  action_id: string | null;
  receipt_id: string | null;
  kind: string;
  source: string;
  summary: string;
  artifact_id: string | null;
  checksum: string | null;
  created_at: number;
};

export type TimelineEntry = {
  action: ActionRecord;
  execution: ExecutionRecord | null;
  receipt: ReceiptRecord | null;
  artifacts: ArtifactRecord[];
};

export type AiSettings = {
  enabled: boolean;
  provider: "openai-compatible";
  base_url: string;
  model: string;
  timeout_secs: number;
  max_tokens: number;
  temperature: number;
};

export type UiSettings = {
  density: "comfortable" | "compact";
  font_size: "small" | "default" | "large";
  mono_font_size: 12 | 13 | 14 | 15;
  animations: boolean;
  reduced_motion: boolean;
  sidebar_mode: "expanded" | "compact";
  remember_sidebar_width: boolean;
  sidebar_width: number;
  remember_inspector_width: boolean;
  inspector_width: number;
  remember_inspector_state: boolean;
  inspector_open: boolean;
  zoom: 90 | 100 | 110 | 125;
};

export type BehaviorSettings = {
  restore_last_project: boolean;
  restore_last_workspace: boolean;
  auto_open_investigation: boolean;
  auto_scroll_logs: boolean;
  open_logs_on_failure: boolean;
  notifications: boolean;
};

export type WorkspaceSettings = {
  kind: "root" | "project" | "session";
  root_view: RootView;
  project_path: string | null;
  project_tab: ProjectTab;
  investigation_case_id: string | null;
  session_id: string | null;
  session_tab: WorkspaceTab;
};

export type AppSettings = {
  language: "en" | "ru";
  theme: "system" | "dark" | "light";
  ui: UiSettings;
  behavior: BehaviorSettings;
  workspace: WorkspaceSettings;
  ai: AiSettings;
};

export const DEFAULT_SETTINGS: AppSettings = {
  language: "en",
  theme: "system",
  ui: {
    density: "comfortable",
    font_size: "default",
    mono_font_size: 13,
    animations: true,
    reduced_motion: false,
    sidebar_mode: "expanded",
    remember_sidebar_width: true,
    sidebar_width: 256,
    remember_inspector_width: true,
    inspector_width: 480,
    remember_inspector_state: true,
    inspector_open: true,
    zoom: 100,
  },
  behavior: {
    restore_last_project: true,
    restore_last_workspace: true,
    auto_open_investigation: true,
    auto_scroll_logs: true,
    open_logs_on_failure: true,
    notifications: true,
  },
  workspace: {
    kind: "root",
    root_view: "home",
    project_path: null,
    project_tab: "project-overview",
    investigation_case_id: null,
    session_id: null,
    session_tab: "overview",
  },
  ai: {
    enabled: false,
    provider: "openai-compatible",
    base_url: "http://localhost:1234/v1",
    model: "",
    timeout_secs: 60,
    max_tokens: 2048,
    temperature: 0.2,
  },
};

export type CapsuleSummary = {
  session_id: string;
  title: string;
  version: number;
  created_at: number;
  file_count: number;
  total_uncompressed_bytes: number;
  redactions: string[];
};

export type CapsuleFile = {
  path: string;
  sha256: string;
  size: number;
  media_type: string;
};

export type CapsuleExportPreview = {
  summary: CapsuleSummary;
  files: CapsuleFile[];
};

export type ImportedCapsule = {
  id: string;
  source_path: string;
  stored_path: string;
  session_id: string | null;
  title: string | null;
  format_version: number;
  sha256: string;
  imported_at: number;
};

export type GitHubStatus = {
  installed: boolean;
  authenticated: boolean;
  version: string | null;
  account_hint: string | null;
};

export type GitHubCreatedItem = { url: string; kind: string };
export type AiConnectionStatus = { reachable: boolean; model: string; provider: string };

export type WorkspaceTab = "overview" | "timeline" | "evidence" | "changes" | "environment" | "verification";
export type RootView = "home" | "projects" | "sessions" | "capsules" | "settings";

export type ProjectGitState = {
  root_path: string;
  branch: string;
  head_commit: string | null;
  is_dirty: boolean;
  changed_files: string[];
};

export type LanguageStat = { language: string; files: number };
export type TechnologySignal = { name: string; category: string; evidence: string[] };
export type ProjectCommandKind = "Build" | "Test" | "Lint" | "Typecheck" | "Dev" | "Check" | "Other";
export type CommandConfidence = "Declared" | "Conventional";
export type ProjectCommand = {
  id: string;
  label: string;
  kind: ProjectCommandKind;
  executable: string;
  args: string[];
  source: string;
  confidence: CommandConfidence;
};
export type ProjectSignalSeverity = "Info" | "Review" | "Warning";
export type ProjectSignal = {
  id: string;
  severity: ProjectSignalSeverity;
  title: string;
  detail: string;
  evidence: string[];
};
export type ProjectStats = {
  files_seen: number;
  source_files: number;
  test_files: number;
  documentation_files: number;
  sensitive_files_excluded: number;
  skipped_large_files: number;
  todo_markers: number;
  scan_truncated: boolean;
};
export type ProjectProfile = {
  schema_version: number;
  fingerprint: string;
  root_path: string;
  name: string;
  version: string | null;
  description: string | null;
  analyzed_at: number;
  git: ProjectGitState | null;
  languages: LanguageStat[];
  technologies: TechnologySignal[];
  commands: ProjectCommand[];
  entrypoints: string[];
  test_paths: string[];
  documentation: string[];
  ci_files: string[];
  signals: ProjectSignal[];
  stats: ProjectStats;
};

export type HealthRunStatus = "Clean" | "ProblemsFound" | "Incomplete" | "OriginalChanged";
export type HealthCheckStatus = "Passed" | "Failed" | "Blocked" | "TimedOut" | "Error";
export type HealthCheckResult = {
  id: string;
  command_id: string;
  label: string;
  kind: ProjectCommandKind;
  executable: string;
  args: string[];
  status: HealthCheckStatus;
  exit_code: number | null;
  duration_ms: number;
  stdout_preview: string;
  stderr_preview: string;
  stdout_truncated: boolean;
  stderr_truncated: boolean;
  evidence_id: string;
  summary: string;
};
export type ProjectProblemStatus = "Signal" | "Suspected" | "Reproduced" | "RootCaused" | "FixProposed" | "Verified" | "Applied" | "Dismissed";
export type ProjectProblemRecord = {
  id: string;
  problem_key: string;
  root_path: string;
  status: ProjectProblemStatus;
  active: boolean;
  title: string;
  summary: string;
  command_id: string;
  health_run_id: string;
  check_run_id: string;
  evidence_ids: string[];
  first_seen_at: number;
  last_seen_at: number;
  cleared_at: number | null;
  occurrences: number;
};
export type ProjectHealthReport = {
  id: string;
  root_path: string;
  project_name: string;
  base_commit: string;
  started_at: number;
  finished_at: number;
  status: HealthRunStatus;
  original_unchanged: boolean;
  source_had_local_changes: boolean;
  checks: HealthCheckResult[];
  problems: ProjectProblemRecord[];
};

export type PlanStage = "Diagnostics" | "Tests" | "Build";
export type RelativeCost = "Low" | "Medium" | "High";
export type PlannedCheck = {
  order: number;
  command_id: string;
  label: string;
  kind: ProjectCommandKind;
  executable: string;
  args: string[];
  stage: PlanStage;
  cost: RelativeCost;
  reason_code: string;
  source: string;
  confidence: CommandConfidence;
  after: string[];
};
export type PlanOmission = { command_id: string; label: string; reason_code: string };
export type PlanNotice = { code: string; detail: string };
export type BugHunterPlan = {
  strategy: string;
  project_name: string;
  project_fingerprint: string;
  steps: PlannedCheck[];
  omitted: PlanOmission[];
  notices: PlanNotice[];
};
export type FailureClass = "Compilation" | "TypeSystem" | "Test" | "Lint" | "Build" | "Timeout" | "Execution" | "Unknown";
export type InvestigationExperiment = {
  order: number;
  kind: string;
  title: string;
  purpose: string;
  command_id: string | null;
  requires_evidence: boolean;
};
export type FailureCluster = {
  id: string;
  class: FailureClass;
  signature: string;
  title: string;
  summary: string;
  check_ids: string[];
  command_ids: string[];
  evidence_ids: string[];
  related_problem_ids: string[];
  investigation_query: string;
  experiments: InvestigationExperiment[];
};
export type ExecutionBlocker = { command_id: string; label: string; summary: string; evidence_id: string };
export type BugHunterAnalysis = {
  health_run_id: string;
  clusters: FailureCluster[];
  blockers: ExecutionBlocker[];
  failed_checks: number;
  clustered_failures: number;
};

export type ContextStats = {
  files_considered: number;
  files_ranked: number;
  sensitive_files_excluded: number;
  skipped_large_or_binary: number;
  selected_chars: number;
  candidate_scan_truncated: boolean;
  packet_truncated: boolean;
};
export type ContextSnippet = {
  id: string;
  path: string;
  language: string;
  score: number;
  reasons: string[];
  line_start: number;
  line_end: number;
  content: string;
  checksum: string;
  truncated: boolean;
};
export type ContextPacket = {
  root_path: string;
  query: string;
  snippets: ContextSnippet[];
  stats: ContextStats;
};

export type ProjectTab = "project-overview" | "problems" | "agent" | "checks";

export type InvestigationState = "HypothesisRequired" | "ExperimentRequired" | "HypothesisSupported" | "Archived";
export type HypothesisStatus = "Proposed" | "Supported" | "Contradicted" | "Inconclusive";
export type HypothesisSource = "Manual" | "Model";
export type ExperimentConclusion = "SupportsHypothesis" | "DoesNotSupport" | "Inconclusive" | "OriginalChanged" | "WorkspaceMutatedByCommand";
export type InvestigationCriterion = {
  command_id: string;
  label: string;
  kind: ProjectCommandKind;
  executable: string;
  args: string[];
  expected_exit_code: number;
  baseline_status: HealthCheckStatus;
  baseline_exit_code: number | null;
  baseline_evidence_id: string;
  baseline_summary: string;
  baseline_stdout_preview: string;
  baseline_stderr_preview: string;
  baseline_duration_ms: number;
  baseline_finished_at: number;
};
export type SourceEvidenceRef = {
  id: string;
  path: string;
  line_start: number;
  line_end: number;
  checksum: string;
  reasons: string[];
  language: string;
  score: number;
  excerpt: string;
  truncated: boolean;
};
export type HypothesisDraft = {
  statement: string;
  rationale: string;
  supporting_evidence_ids: string[];
  neutral_evidence_ids: string[];
  contradicting_evidence_ids: string[];
  falsifier: string;
  next_experiment: string;
  confidence_percent: number;
  source: HypothesisSource;
};
export type InvestigationHypothesis = {
  id: string;
  statement: string;
  rationale: string;
  supporting_evidence_ids: string[];
  neutral_evidence_ids: string[];
  contradicting_evidence_ids: string[];
  rejected_evidence_ids: string[];
  falsifier: string;
  next_experiment: string;
  requested_confidence_percent: number;
  accepted_confidence_percent: number;
  source: HypothesisSource;
  status: HypothesisStatus;
};
export type CausalExperimentRecord = {
  id: string;
  hypothesis_id: string;
  command_id: string;
  started_at: number;
  finished_at: number;
  intervention_sha256: string;
  changed_files: string[];
  exit_code: number | null;
  status: HealthCheckStatus;
  stdout_preview: string;
  stderr_preview: string;
  evidence_id: string;
  original_unchanged: boolean;
  workspace_unchanged_by_command: boolean;
  conclusion: ExperimentConclusion;
};
export type InvestigationCase = {
  schema_version: number;
  id: string;
  root_path: string;
  repo_root: string;
  project_relative_path: string;
  project_name: string;
  health_run_id: string;
  cluster: FailureCluster;
  base_commit: string;
  state: InvestigationState;
  criterion: InvestigationCriterion;
  evidence_ids: string[];
  source_evidence: SourceEvidenceRef[];
  hypotheses: InvestigationHypothesis[];
  experiments: CausalExperimentRecord[];
  created_at: number;
  updated_at: number;
};
export type FixWorkspaceRecord = {
  case_id: string;
  repo_root: string;
  project_path: string;
  base_commit: string;
  branch: string;
  worktree_path: string;
  original_head: string;
  original_branch: string;
  dirty: boolean;
  changed_files: string[];
  created_at: number;
  updated_at: number;
};
export type FixWorkspaceDiff = { patch: string; files: string[] };

export type PatchIdentity = {
  source_commit: string;
  source_state_sha256: string;
  shadow_commit: string;
  patch_sha256: string;
  patch_size: number;
  files: string[];
};
export type RegressionLevel = "Required" | "Recommended" | "Optional";
export type RegressionDraft = {
  stable_id: string;
  title: string;
  executable: string;
  args: string[];
  expected_exit_code: number;
  level: RegressionLevel;
};
export type RegressionCheck = RegressionDraft & {
  id: string;
  session_id: string;
  status: string;
  receipt_id: string | null;
  verified_patch_sha256: string | null;
  created_at: number;
  updated_at: number;
};
export type VerificationHandoff = {
  session_id: string;
  investigation_case_id: string;
  hypothesis_id: string;
  experiment_id: string;
  source_commit: string;
  patch_sha256: string;
  patch_size: number;
  files: string[];
  activated_at: number | null;
  created_at: number;
};
export type VerificationProof = {
  session_id: string;
  step_id: string;
  cycle: number;
  identity: PatchIdentity;
  criterion_sha256: string;
  command_sha256: string;
  after_run_id: string;
  verified_at: number;
};
export type VerificationStatus = {
  outcome: string;
  ready_to_apply: boolean;
  reason_code: string;
  message: string;
  current_identity: PatchIdentity | null;
  proof: VerificationProof | null;
  handoff: VerificationHandoff | null;
  regressions: RegressionCheck[];
  required_passed: number;
  required_total: number;
};

export type RecoveryEntry = {
  id: string;
  repo_path: string;
  base_commit: string;
  worktree_path: string;
  branch: string;
  ts: number;
  state: "Active" | "Applied" | "AppliedCleanupPending" | "Discarded" | "CleanupFailed";
  last_error: string | null;
};
