import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

type Session = {
  id: string;
  repo_id: string | null;
  created_at: number;
  updated_at: number | null;
  state: string;
  meta: string | null;
};

type RepositoryInfo = {
  id: string | null;
  path: string;
  head_commit: string;
  branch: string;
  is_dirty: boolean;
};

type Action = {
  id: string;
  kind: string;
  state: string;
  meta: string | null;
  created_at: number;
};

type Execution = {
  id: string;
  action_id: string;
  status: string;
  started_at: number;
  finished_at: number | null;
  duration_ms: number | null;
};

type Receipt = {
  id: string;
  execution_id: string;
  summary: string | null;
  stdout_preview: string | null;
  stderr_preview: string | null;
  stdout_truncated: boolean;
  stderr_truncated: boolean;
  created_at: number;
};

type Artifact = {
  id: string;
  receipt_id: string;
  checksum: string;
  size: number;
  media_type: string | null;
};

type TimelineEntry = {
  action: Action;
  execution: Execution | null;
  receipt: Receipt | null;
  artifacts: Artifact[];
};

type Contract = {
  id: string;
  session_id: string;
  title: string;
  description: string | null;
  state: string;
  version: number;
  created_at: number;
};

type OutcomeCheckSummary = {
  check_id: string;
  stable_id: string;
  description: string;
  required: boolean;
  before: string | null;
  after: string | null;
  outcome: string;
};

type OutcomeSummary = {
  contract_id: string;
  overall: string;
  checks: OutcomeCheckSummary[];
};

type BridgeError = {
  code?: string;
  message?: string;
};

type ActionMeta = {
  phase?: string;
  expected_exit_code?: number;
  command?: {
    executable?: string;
    args?: string[];
    cwd?: string | null;
  };
};

type WorkspaceView = "Timeline" | "Verification";
type InspectorTab = "Details" | "Output" | "Evidence";

const formatTime = (seconds: number) =>
  new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(new Date(seconds * 1000));

const formatRelative = (seconds: number) => {
  const delta = Math.max(0, Math.floor(Date.now() / 1000 - seconds));
  if (delta < 60) return `${delta}s ago`;
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`;
  if (delta < 86400) return `${Math.floor(delta / 3600)}h ago`;
  return `${Math.floor(delta / 86400)}d ago`;
};

const formatDuration = (durationMs: number | null) => {
  if (durationMs === null) return "—";
  if (durationMs < 1000) return `${durationMs} ms`;
  return `${(durationMs / 1000).toFixed(durationMs < 10_000 ? 2 : 1)} s`;
};

const formatBytes = (bytes: number) => {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
};

const humanize = (value: string) =>
  value
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[_-]+/g, " ")
    .replace(/^./, (char) => char.toUpperCase());

const repositoryName = (path: string) => {
  const normalized = path.replace(/\\/g, "/").replace(/\/+$/, "");
  return normalized.split("/").pop() || normalized;
};

function bridgeMessage(error: unknown) {
  if (typeof error === "string") return error;
  if (error && typeof error === "object") {
    const bridge = error as BridgeError;
    if (bridge.message) return bridge.message;
  }
  return "ReproDeck could not complete that operation.";
}

function statusTone(value: string) {
  const normalized = value.toLowerCase();
  if (normalized.includes("pass") || normalized.includes("success") || normalized.includes("verified")) return "success";
  if (normalized.includes("fail") || normalized.includes("error") || normalized.includes("denied")) return "danger";
  if (normalized.includes("interrupt") || normalized.includes("pending") || normalized.includes("running")) return "warning";
  return "neutral";
}

function parseActionMeta(meta: string | null): ActionMeta | null {
  if (!meta) return null;
  try {
    const parsed: unknown = JSON.parse(meta);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return null;
    return parsed as ActionMeta;
  } catch {
    return null;
  }
}

function commandLabel(meta: ActionMeta | null) {
  const executable = meta?.command?.executable?.trim();
  if (!executable) return null;
  const args = Array.isArray(meta?.command?.args) ? meta.command.args.filter((arg) => typeof arg === "string") : [];
  return [executable, ...args].join(" ");
}

function entryStatus(entry: TimelineEntry) {
  return entry.execution?.status ?? entry.action.state;
}

function entryTitle(entry: TimelineEntry) {
  const meta = parseActionMeta(entry.action.meta);
  if (meta?.phase) return `${humanize(meta.phase)} · ${humanize(entry.action.kind)}`;
  return humanize(entry.action.kind);
}

function Mark({ tone = "neutral" }: { tone?: string }) {
  return <span className={`mark mark-${tone}`} aria-hidden="true" />;
}

function App() {
  const [sessions, setSessions] = useState<Session[]>([]);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null);
  const [repository, setRepository] = useState<RepositoryInfo | null>(null);
  const [timelineEntries, setTimelineEntries] = useState<TimelineEntry[]>([]);
  const [contracts, setContracts] = useState<Contract[]>([]);
  const [selectedActionId, setSelectedActionId] = useState<string | null>(null);
  const [selectedContractId, setSelectedContractId] = useState<string | null>(null);
  const [summary, setSummary] = useState<OutcomeSummary | null>(null);
  const [view, setView] = useState<WorkspaceView>("Timeline");
  const [inspectorTab, setInspectorTab] = useState<InspectorTab>("Details");
  const [search, setSearch] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [newSessionOpen, setNewSessionOpen] = useState(false);
  const [newSessionId, setNewSessionId] = useState("");
  const [creating, setCreating] = useState(false);
  const [repositoryOpen, setRepositoryOpen] = useState(false);
  const [repositoryPath, setRepositoryPath] = useState("");
  const [attachingRepository, setAttachingRepository] = useState(false);

  const selectedSession = sessions.find((session) => session.id === selectedSessionId) ?? null;
  const selectedEntry = timelineEntries.find((entry) => entry.action.id === selectedActionId) ?? null;
  const selectedContract = contracts.find((contract) => contract.id === selectedContractId) ?? null;
  const selectedMeta = selectedEntry ? parseActionMeta(selectedEntry.action.meta) : null;
  const selectedCommand = commandLabel(selectedMeta);

  const filteredSessions = useMemo(() => {
    const query = search.trim().toLowerCase();
    if (!query) return sessions;
    return sessions.filter((session) => session.id.toLowerCase().includes(query) || session.state.toLowerCase().includes(query));
  }, [search, sessions]);

  const timeline = useMemo(() => [...timelineEntries].reverse(), [timelineEntries]);

  async function loadSession(id: string) {
    setSelectedSessionId(id);
    setSelectedActionId(null);
    setSelectedContractId(null);
    setSummary(null);
    setInspectorTab("Details");
    setError(null);
    try {
      const [nextTimeline, nextContracts, nextRepository] = await Promise.all([
        invoke<TimelineEntry[]>("list_timeline_entries", { sessionId: id }),
        invoke<Contract[]>("list_contracts", { sessionId: id }),
        invoke<RepositoryInfo | null>("get_session_repository", { sessionId: id }),
      ]);
      setTimelineEntries(nextTimeline ?? []);
      setContracts(nextContracts ?? []);
      setRepository(nextRepository ?? null);
      if (nextRepository?.path) setRepositoryPath(nextRepository.path);
    } catch (nextError) {
      setError(bridgeMessage(nextError));
      setTimelineEntries([]);
      setContracts([]);
      setRepository(null);
    }
  }

  async function refreshSessions(selectNewest = false) {
    setError(null);
    try {
      const nextSessions = await invoke<Session[]>("list_sessions");
      setSessions(nextSessions ?? []);
      const target = selectNewest ? nextSessions?.[0]?.id : selectedSessionId ?? nextSessions?.[0]?.id;
      if (target) await loadSession(target);
    } catch (nextError) {
      setError(bridgeMessage(nextError));
    } finally {
      setLoading(false);
    }
  }

  async function createSession() {
    const id = newSessionId.trim();
    if (!id) return;
    setCreating(true);
    setError(null);
    try {
      await invoke<Session>("create_session", { id });
      setNewSessionId("");
      setNewSessionOpen(false);
      await refreshSessions(true);
    } catch (nextError) {
      setError(bridgeMessage(nextError));
    } finally {
      setCreating(false);
    }
  }

  async function attachRepository() {
    if (!selectedSessionId || !repositoryPath.trim()) return;
    setAttachingRepository(true);
    setError(null);
    try {
      const attached = await invoke<RepositoryInfo>("attach_repository", {
        sessionId: selectedSessionId,
        path: repositoryPath.trim(),
      });
      setRepository(attached);
      setSessions((current) => current.map((session) => session.id === selectedSessionId ? { ...session, repo_id: attached.id } : session));
      setRepositoryPath(attached.path);
      setRepositoryOpen(false);
    } catch (nextError) {
      setError(bridgeMessage(nextError));
    } finally {
      setAttachingRepository(false);
    }
  }

  async function inspectContract(contractId: string) {
    setSelectedContractId(contractId);
    setError(null);
    try {
      const nextSummary = await invoke<OutcomeSummary>("get_outcome_summary", { contractId });
      setSummary(nextSummary);
    } catch (nextError) {
      setSummary(null);
      setError(bridgeMessage(nextError));
    }
  }

  function selectTimelineEntry(actionId: string) {
    setSelectedActionId(actionId);
    setInspectorTab("Details");
  }

  useEffect(() => {
    void refreshSessions();
    // Initial bridge load is intentionally one-shot; later refreshes are explicit.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="shell">
      <header className="topbar">
        <div className="brand-block">
          <span className="brand-mark">RD</span>
          <strong>REPRODECK</strong>
        </div>
        <div className="workspace-crumbs">
          <button className="repo-chip repo-button" onClick={() => selectedSession && setRepositoryOpen(true)} disabled={!selectedSession} title={repository?.path || "Attach a Git repository"}>
            {repository ? repositoryName(repository.path) : "Attach repository"}
          </button>
          {repository && <span className="branch-chip">⌘ {repository.branch}</span>}
          {repository?.is_dirty && <span className="dirty-chip">DIRTY</span>}
          <span className="isolation-chip"><i />LOCAL-FIRST</span>
        </div>
        <label className="search-box">
          <span>⌕</span>
          <input value={search} onChange={(event) => setSearch(event.target.value)} placeholder="Search sessions" />
          <kbd>Ctrl K</kbd>
        </label>
      </header>

      <div className="workspace-grid">
        <aside className="sidebar">
          <div className="sidebar-action-wrap">
            <button className="new-session-button" onClick={() => setNewSessionOpen(true)}><span>＋</span>New bug session</button>
          </div>

          <nav className="nav-block" aria-label="Workspace navigation">
            <p className="section-label">Workspace</p>
            <button onClick={() => selectedSession && setRepositoryOpen(true)} disabled={!selectedSession}><span className="nav-icon">◇</span>Repository <em>{repository ? "1" : ""}</em></button>
            <button><span className="nav-icon">◉</span>Sessions <em>{sessions.length}</em></button>
            <button className={view === "Timeline" ? "active" : ""} onClick={() => setView("Timeline")}><span className="nav-icon">◷</span>Timeline <em>{timelineEntries.length || ""}</em></button>
            <button className={view === "Verification" ? "active" : ""} onClick={() => setView("Verification")}><span className="nav-icon">✓</span>Verification <em>{contracts.length || ""}</em></button>
            <button disabled><span className="nav-icon">↔</span>Changes <small>next</small></button>
            <button disabled><span className="nav-icon">▤</span>Evidence <small>next</small></button>
            <button disabled><span className="nav-icon">↶</span>History <small>next</small></button>
          </nav>

          <div className="sidebar-divider" />
          <section className="recent-block">
            <p className="section-label">Recent sessions</p>
            <div className="recent-scroll">
              {filteredSessions.map((session) => (
                <button key={session.id} className={session.id === selectedSessionId ? "recent-session selected" : "recent-session"} onClick={() => void loadSession(session.id)}>
                  <div><strong>{session.id}</strong><Mark tone={statusTone(session.state)} /></div>
                  <span>{session.state} · {formatRelative(session.updated_at ?? session.created_at)}</span>
                </button>
              ))}
              {!loading && filteredSessions.length === 0 && <p className="muted-empty">No matching sessions.</p>}
            </div>
          </section>
          <footer className="sidebar-footer"><span>♢</span>Local-only · no telemetry</footer>
        </aside>

        <main className="content-pane">
          {error && <div className="error-banner"><span>!</span>{error}<button onClick={() => setError(null)}>×</button></div>}

          {!selectedSession ? (
            <section className="empty-workspace">
              <div className="empty-logo">RD</div>
              <p className="eyebrow">Capture · Reproduce · Fix · Prove</p>
              <h1>{loading ? "Opening ReproDeck…" : "Start with a bug session"}</h1>
              <p>Create a local session first. ReproDeck will keep verification history and evidence in its own local storage.</p>
              {!loading && <button className="primary-button" onClick={() => setNewSessionOpen(true)}>＋ New bug session</button>}
            </section>
          ) : (
            <>
              <section className="session-header">
                <div>
                  <div className="title-row"><h1>{selectedSession.id}</h1><span className={`state-pill tone-${statusTone(selectedSession.state)}`}>{selectedSession.state}</span></div>
                  <p>{repository ? `${repository.path} · HEAD ${repository.head_commit.slice(0, 8)}` : "No repository attached yet"}</p>
                </div>
                <div className="header-actions">
                  <button className="ghost-button" onClick={() => void refreshSessions()}>↻ Refresh</button>
                  {!repository && <button className="primary-button" onClick={() => setRepositoryOpen(true)}>◇ Attach repository</button>}
                  <button className="apply-button" disabled title="Apply becomes available after the isolated shadow workspace is wired">✓ Apply changes</button>
                </div>
              </section>

              {!repository && (
                <div className="repo-banner">
                  <div><span>◇</span><div><strong>Attach the Git repository for this bug session</strong><p>ReproDeck will resolve the real repository root and HEAD locally. It will not modify the repository during attachment.</p></div></div>
                  <button className="ghost-button" onClick={() => setRepositoryOpen(true)}>Choose path</button>
                </div>
              )}

              <div className="tabbar">
                <button className={view === "Timeline" ? "active" : ""} onClick={() => setView("Timeline")}>Timeline</button>
                <button className={view === "Verification" ? "active" : ""} onClick={() => setView("Verification")}>Verification</button>
                <button disabled>Changes <span>—</span></button>
                <button disabled>Evidence <span>—</span></button>
              </div>

              {view === "Timeline" ? (
                <section className="scroll-content">
                  <div className="content-kicker"><span>Execution timeline</span><span>{timeline.length} events</span></div>
                  {timeline.length === 0 ? (
                    <div className="panel-empty"><strong>No timeline events yet</strong><p>Actions created by reproduction and verification flows will appear here automatically.</p></div>
                  ) : (
                    <div className="timeline-list">
                      {timeline.map((entry) => {
                        const status = entryStatus(entry);
                        const tone = statusTone(status);
                        const meta = parseActionMeta(entry.action.meta);
                        const command = commandLabel(meta);
                        return (
                          <button key={entry.action.id} className={selectedActionId === entry.action.id ? "timeline-row selected" : "timeline-row"} onClick={() => selectTimelineEntry(entry.action.id)}>
                            <time>{formatTime(entry.action.created_at)}</time>
                            <span className="timeline-node"><Mark tone={tone} /></span>
                            <div className="timeline-copy">
                              <div><strong>{entryTitle(entry)}</strong><span className={`mini-pill tone-${tone}`}>{status}</span></div>
                              <p>{command ?? entry.receipt?.summary ?? entry.action.id}</p>
                            </div>
                          </button>
                        );
                      })}
                    </div>
                  )}
                </section>
              ) : (
                <section className="scroll-content">
                  <div className="content-kicker"><span>Outcome verification</span><span>{contracts.length} contracts</span></div>
                  {contracts.length === 0 ? (
                    <div className="panel-empty"><strong>No outcome contract yet</strong><p>Once a reproduction defines expected BEFORE and AFTER checks, the verdict will appear here without TypeScript rebuilding business logic.</p></div>
                  ) : (
                    <div className="contract-grid">
                      {contracts.map((contract) => (
                        <button key={contract.id} className={selectedContractId === contract.id ? "contract-card selected" : "contract-card"} onClick={() => void inspectContract(contract.id)}>
                          <div className="contract-top"><span className="contract-icon">✓</span><span className={`mini-pill tone-${statusTone(contract.state)}`}>{contract.state}</span></div>
                          <strong>{contract.title}</strong>
                          <p>{contract.description || "Verification contract"}</p>
                          <footer><span>v{contract.version}</span><span>{formatRelative(contract.created_at)}</span></footer>
                        </button>
                      ))}
                    </div>
                  )}

                  {summary && (
                    <section className={`verification-result tone-border-${statusTone(summary.overall)}`}>
                      <div className="result-heading"><div><p className="eyebrow">Verification verdict</p><h2>{humanize(summary.overall)}</h2></div><span className={`verdict-badge tone-${statusTone(summary.overall)}`}>{humanize(summary.overall)}</span></div>
                      <div className="checks-table">
                        <div className="check-row check-head"><span>Check</span><span>Before</span><span>After</span><span>Result</span></div>
                        {summary.checks.map((check) => (
                          <div className="check-row" key={check.check_id}>
                            <span><strong>{check.description}</strong><small>{check.required ? "Required" : "Optional"} · {check.stable_id}</small></span>
                            <span className={`status-text tone-${statusTone(check.before ?? "pending")}`}>{check.before ?? "—"}</span>
                            <span className={`status-text tone-${statusTone(check.after ?? "pending")}`}>{check.after ?? "—"}</span>
                            <span className={`status-text tone-${statusTone(check.outcome)}`}>{humanize(check.outcome)}</span>
                          </div>
                        ))}
                      </div>
                    </section>
                  )}
                </section>
              )}
            </>
          )}
        </main>

        <aside className="inspector">
          <div className="inspector-heading"><span>Selected item</span><strong>{view === "Timeline" ? "Receipt & evidence" : "Verification details"}</strong></div>
          {view === "Timeline" ? (
            selectedEntry ? (
              <>
                <div className="inspector-tabs">
                  {(["Details", "Output", "Evidence"] as InspectorTab[]).map((tab) => (
                    <button key={tab} className={inspectorTab === tab ? "active" : ""} onClick={() => setInspectorTab(tab)}>{tab}{tab === "Evidence" && selectedEntry.artifacts.length > 0 ? ` ${selectedEntry.artifacts.length}` : ""}</button>
                  ))}
                </div>
                <div className="inspector-body">
                  {inspectorTab === "Details" && (
                    <>
                      <span className={`large-status tone-${statusTone(entryStatus(selectedEntry))}`}>{entryStatus(selectedEntry)}</span>
                      {selectedCommand && <div className="detail-block"><span className="detail-label">Command</span><code className="command-box">{selectedCommand}</code></div>}
                      <dl>
                        <div><dt>Type</dt><dd>{entryTitle(selectedEntry)}</dd></div>
                        {selectedMeta?.phase && <div><dt>Phase</dt><dd>{selectedMeta.phase}</dd></div>}
                        {typeof selectedMeta?.expected_exit_code === "number" && <div><dt>Expected exit</dt><dd>{selectedMeta.expected_exit_code}</dd></div>}
                        {selectedMeta?.command?.cwd && <div><dt>Working dir</dt><dd className="mono breakable">{selectedMeta.command.cwd}</dd></div>}
                        <div><dt>Duration</dt><dd>{formatDuration(selectedEntry.execution?.duration_ms ?? null)}</dd></div>
                        <div><dt>Receipt</dt><dd className="mono breakable">{selectedEntry.receipt?.id ?? "—"}</dd></div>
                        <div><dt>Captured</dt><dd>{new Date(selectedEntry.action.created_at * 1000).toLocaleString()}</dd></div>
                      </dl>
                    </>
                  )}

                  {inspectorTab === "Output" && (
                    <div className="output-stack">
                      <section className="output-section">
                        <div><strong>stdout</strong>{selectedEntry.receipt?.stdout_truncated && <span>truncated</span>}</div>
                        <pre className="output-box">{selectedEntry.receipt?.stdout_preview || "No stdout captured."}</pre>
                      </section>
                      <section className="output-section">
                        <div><strong>stderr</strong>{selectedEntry.receipt?.stderr_truncated && <span>truncated</span>}</div>
                        <pre className="output-box">{selectedEntry.receipt?.stderr_preview || "No stderr captured."}</pre>
                      </section>
                      {!selectedEntry.receipt && <p className="inspector-hint">This action has no completed receipt yet.</p>}
                    </div>
                  )}

                  {inspectorTab === "Evidence" && (
                    selectedEntry.artifacts.length > 0 ? (
                      <div className="artifact-list">
                        {selectedEntry.artifacts.map((artifact) => (
                          <article className="artifact-row" key={artifact.id}>
                            <div className="artifact-icon">▤</div>
                            <div><strong>{artifact.media_type || "Evidence artifact"}</strong><span>{formatBytes(artifact.size)} · {artifact.checksum.slice(0, 12)}…</span><code>{artifact.id}</code></div>
                          </article>
                        ))}
                      </div>
                    ) : <div className="inspector-empty compact">No evidence artifacts are linked to this receipt.</div>
                  )}
                </div>
              </>
            ) : <div className="inspector-empty">Select a timeline event to inspect its execution, receipt output, and evidence.</div>
          ) : (
            selectedContract ? (
              <div className="inspector-body">
                <span className={`large-status tone-${statusTone(summary?.overall ?? selectedContract.state)}`}>{summary ? humanize(summary.overall) : selectedContract.state}</span>
                <dl>
                  <div><dt>Contract</dt><dd>{selectedContract.title}</dd></div>
                  <div><dt>Version</dt><dd>v{selectedContract.version}</dd></div>
                  <div><dt>Checks</dt><dd>{summary?.checks.length ?? "Load to inspect"}</dd></div>
                </dl>
                {summary ? <div className="inspector-note success-note"><strong>Backend-evaluated verdict.</strong><p>The UI renders the typed result returned by Rust. BEFORE/AFTER semantics remain in the core rather than being reconstructed in React.</p></div> : <button className="primary-button full" onClick={() => void inspectContract(selectedContract.id)}>Evaluate contract</button>}
              </div>
            ) : <div className="inspector-empty">Select an outcome contract to inspect the BEFORE → AFTER verdict.</div>
          )}
        </aside>
      </div>

      <div className="statusbar">
        <span>● Core storage ready</span>
        <span>{repository ? `${repositoryName(repository.path)} · ${repository.branch} · ${repository.is_dirty ? "dirty" : "clean"}` : selectedSession ? "Repository not attached" : "No session selected"}</span>
        <span>{repository ? `HEAD ${repository.head_commit.slice(0, 8)}` : "Esc back"}</span>
      </div>

      {newSessionOpen && (
        <div className="modal-backdrop" role="presentation" onMouseDown={() => !creating && setNewSessionOpen(false)}>
          <section className="modal" role="dialog" aria-modal="true" aria-labelledby="new-session-title" onMouseDown={(event) => event.stopPropagation()}>
            <header><span className="modal-icon">＋</span><div><h2 id="new-session-title">New bug session</h2><p>Create the local session first. You can attach its Git repository immediately after creation.</p></div></header>
            <label><span>Session ID</span><input autoFocus value={newSessionId} onChange={(event) => setNewSessionId(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") void createSession(); }} placeholder="auth-refresh-regression" /></label>
            <footer><button className="ghost-button" disabled={creating} onClick={() => setNewSessionOpen(false)}>Cancel</button><button className="primary-button" disabled={creating || !newSessionId.trim()} onClick={() => void createSession()}>{creating ? "Creating…" : "Create session"}</button></footer>
          </section>
        </div>
      )}

      {repositoryOpen && selectedSession && (
        <div className="modal-backdrop" role="presentation" onMouseDown={() => !attachingRepository && setRepositoryOpen(false)}>
          <section className="modal" role="dialog" aria-modal="true" aria-labelledby="repository-title" onMouseDown={(event) => event.stopPropagation()}>
            <header><span className="modal-icon">◇</span><div><h2 id="repository-title">Attach Git repository</h2><p>Enter a local path. ReproDeck only inspects the repository and records its canonical root, branch and HEAD during this step.</p></div></header>
            <label><span>Repository path</span><input autoFocus value={repositoryPath} onChange={(event) => setRepositoryPath(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") void attachRepository(); }} placeholder="C:\\Users\\you\\Projects\\my-app" /></label>
            {repository && <p className="repo-path-caption">Currently attached: <span>{repository.path}</span></p>}
            <footer><button className="ghost-button" disabled={attachingRepository} onClick={() => setRepositoryOpen(false)}>Cancel</button><button className="primary-button" disabled={attachingRepository || !repositoryPath.trim()} onClick={() => void attachRepository()}>{attachingRepository ? "Inspecting…" : repository ? "Reattach" : "Attach repository"}</button></footer>
          </section>
        </div>
      )}
    </div>
  );
}

export default App;
