import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

type Session = {
  id: string;
  created_at: number;
  updated_at: number | null;
  state: string;
  meta: string | null;
};

type Action = {
  id: string;
  kind: string;
  state: string;
  created_at: number;
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

type WorkspaceView = "Timeline" | "Verification";

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

const humanize = (value: string) =>
  value
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[_-]+/g, " ")
    .replace(/^./, (char) => char.toUpperCase());

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

function Mark({ tone = "neutral" }: { tone?: string }) {
  return <span className={`mark mark-${tone}`} aria-hidden="true" />;
}

function App() {
  const [sessions, setSessions] = useState<Session[]>([]);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null);
  const [actions, setActions] = useState<Action[]>([]);
  const [contracts, setContracts] = useState<Contract[]>([]);
  const [selectedActionId, setSelectedActionId] = useState<string | null>(null);
  const [selectedContractId, setSelectedContractId] = useState<string | null>(null);
  const [summary, setSummary] = useState<OutcomeSummary | null>(null);
  const [view, setView] = useState<WorkspaceView>("Timeline");
  const [search, setSearch] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [newSessionOpen, setNewSessionOpen] = useState(false);
  const [newSessionId, setNewSessionId] = useState("");
  const [creating, setCreating] = useState(false);

  const selectedSession = sessions.find((session) => session.id === selectedSessionId) ?? null;
  const selectedAction = actions.find((action) => action.id === selectedActionId) ?? null;
  const selectedContract = contracts.find((contract) => contract.id === selectedContractId) ?? null;

  const filteredSessions = useMemo(() => {
    const query = search.trim().toLowerCase();
    if (!query) return sessions;
    return sessions.filter((session) => session.id.toLowerCase().includes(query) || session.state.toLowerCase().includes(query));
  }, [search, sessions]);

  const timeline = useMemo(() => [...actions].reverse(), [actions]);

  async function loadSession(id: string) {
    setSelectedSessionId(id);
    setSelectedActionId(null);
    setSelectedContractId(null);
    setSummary(null);
    setError(null);
    try {
      const [nextActions, nextContracts] = await Promise.all([
        invoke<Action[]>("list_actions", { sessionId: id }),
        invoke<Contract[]>("list_contracts", { sessionId: id }),
      ]);
      setActions(nextActions ?? []);
      setContracts(nextContracts ?? []);
    } catch (nextError) {
      setError(bridgeMessage(nextError));
      setActions([]);
      setContracts([]);
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

  useEffect(() => {
    void refreshSessions();
    // The initial bridge read is intentionally one-shot. Subsequent refreshes are explicit.
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
          <span className="repo-chip">Local workspace</span>
          {selectedSession && <span className="branch-chip">session/{selectedSession.id}</span>}
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
            <button><span className="nav-icon">◇</span>Repository</button>
            <button><span className="nav-icon">◉</span>Sessions <em>{sessions.length}</em></button>
            <button className={view === "Timeline" ? "active" : ""} onClick={() => setView("Timeline")}><span className="nav-icon">◷</span>Timeline <em>{actions.length || ""}</em></button>
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
                  <p>Session · local storage · created {formatRelative(selectedSession.created_at)}</p>
                </div>
                <div className="header-actions">
                  <button className="ghost-button" onClick={() => void refreshSessions()}>↻ Refresh</button>
                  <button className="apply-button" disabled title="Apply UI will be enabled when repository selection is wired to the bridge">✓ Apply changes</button>
                </div>
              </section>

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
                      {timeline.map((action) => {
                        const tone = statusTone(action.state);
                        return (
                          <button key={action.id} className={selectedActionId === action.id ? "timeline-row selected" : "timeline-row"} onClick={() => setSelectedActionId(action.id)}>
                            <time>{formatTime(action.created_at)}</time>
                            <span className="timeline-node"><Mark tone={tone} /></span>
                            <div className="timeline-copy">
                              <div><strong>{humanize(action.kind)}</strong><span className={`mini-pill tone-${tone}`}>{action.state}</span></div>
                              <p>{action.id}</p>
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
          <div className="inspector-heading"><span>Selected item</span><strong>{view === "Timeline" ? "Timeline details" : "Verification details"}</strong></div>
          {view === "Timeline" ? (
            selectedAction ? (
              <div className="inspector-body">
                <span className={`large-status tone-${statusTone(selectedAction.state)}`}>{selectedAction.state}</span>
                <dl>
                  <div><dt>Type</dt><dd>{humanize(selectedAction.kind)}</dd></div>
                  <div><dt>Action ID</dt><dd className="mono breakable">{selectedAction.id}</dd></div>
                  <div><dt>Captured</dt><dd>{new Date(selectedAction.created_at * 1000).toLocaleString()}</dd></div>
                </dl>
                <div className="inspector-note"><strong>Receipt inspector is next.</strong><p>The core already persists executions, receipts and evidence. The next bridge slice will expose those relationships here without duplicating backend logic.</p></div>
              </div>
            ) : <div className="inspector-empty">Select a timeline event to inspect it.</div>
          ) : (
            selectedContract ? (
              <div className="inspector-body">
                <span className={`large-status tone-${statusTone(summary?.overall ?? selectedContract.state)}`}>{summary ? humanize(summary.overall) : selectedContract.state}</span>
                <dl>
                  <div><dt>Contract</dt><dd>{selectedContract.title}</dd></div>
                  <div><dt>Version</dt><dd>v{selectedContract.version}</dd></div>
                  <div><dt>Checks</dt><dd>{summary?.checks.length ?? "Load to inspect"}</dd></div>
                </dl>
                {summary ? <div className="inspector-note success-note"><strong>Backend-evaluated verdict.</strong><p>The UI is rendering the typed outcome returned by Rust; it does not infer BEFORE/AFTER semantics on its own.</p></div> : <button className="primary-button full" onClick={() => void inspectContract(selectedContract.id)}>Evaluate contract</button>}
              </div>
            ) : <div className="inspector-empty">Select an outcome contract to inspect the BEFORE → AFTER verdict.</div>
          )}
        </aside>
      </div>

      <div className="statusbar"><span>● Core storage ready</span><span>{selectedSession ? `Session ${selectedSession.id}` : "No session selected"}</span><span>Esc back</span></div>

      {newSessionOpen && (
        <div className="modal-backdrop" role="presentation" onMouseDown={() => !creating && setNewSessionOpen(false)}>
          <section className="modal" role="dialog" aria-modal="true" aria-labelledby="new-session-title" onMouseDown={(event) => event.stopPropagation()}>
            <header><span className="modal-icon">＋</span><div><h2 id="new-session-title">New bug session</h2><p>Create the local session record now; repository capture will be attached in the next vertical slice.</p></div></header>
            <label><span>Session ID</span><input autoFocus value={newSessionId} onChange={(event) => setNewSessionId(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") void createSession(); }} placeholder="auth-refresh-regression" /></label>
            <footer><button className="ghost-button" disabled={creating} onClick={() => setNewSessionOpen(false)}>Cancel</button><button className="primary-button" disabled={creating || !newSessionId.trim()} onClick={() => void createSession()}>{creating ? "Creating…" : "Create session"}</button></footer>
          </section>
        </div>
      )}
    </div>
  );
}

export default App;
