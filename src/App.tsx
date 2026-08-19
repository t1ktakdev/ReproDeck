import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

type Session = {
  id: string;
  state: string;
  created_at: number;
  updated_at?: number | null;
  meta?: string | null;
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
  description?: string | null;
  state: string;
  version: number;
  created_at: number;
};

type EvaluationResult = {
  verdict?: string | null;
};

function App() {
  const [sessions, setSessions] = useState<Session[]>([]);
  const [selectedSession, setSelectedSession] = useState<string | null>(null);
  const [actions, setActions] = useState<Action[]>([]);
  const [contracts, setContracts] = useState<Contract[]>([]);
  const [verdict, setVerdict] = useState<string | null>(null);

  async function refreshSessions() {
    const s = await invoke<Session[]>("list_sessions");
    setSessions(s || []);
  }

  async function createSession() {
    const id = `s-${Date.now()}`;
    await invoke("create_session", { id });
    refreshSessions();
  }

  async function selectSession(id: string) {
    setSelectedSession(id);
    setVerdict(null);

    const a = await invoke<Action[]>("list_actions", { sessionId: id });
    setActions(a || []);

    const c = await invoke<Contract[]>("list_contracts");
    setContracts((c || []).filter((contract) => contract.session_id === id));
  }

  async function evalContract(contractId: string) {
    const r = await invoke<EvaluationResult>("evaluate_contract", { contractId });
    setVerdict(r?.verdict || null);
  }

  useEffect(() => {
    refreshSessions();
  }, []);

  return (
    <div className="app-root">
      <aside className="sidebar">
        <div className="sidebar-header">
          <h2>Projects / Sessions</h2>
          <button onClick={createSession}>+ New Session</button>
        </div>
        <ul className="session-list">
          {sessions.map((s) => (
            <li key={s.id} onClick={() => selectSession(s.id)} className={s.id === selectedSession ? "selected" : ""}>
              <div className="session-id">{s.id}</div>
              <div className="session-meta">{s.state}</div>
            </li>
          ))}
        </ul>
      </aside>
      <main className="main">
        <header className="main-header">
          <h1>ReproDeck — Session: {selectedSession || "(none)"}</h1>
        </header>
        <section className="pane">
          <h3>Timeline</h3>
          {actions.length === 0 ? <div className="empty">No timeline entries</div> : (
            <ul className="actions-list">
              {actions.map((a) => (
                <li key={a.id}>
                  <div className="a-line"><strong>{a.kind}</strong> — {a.state} — {new Date(a.created_at * 1000).toLocaleString()}</div>
                </li>
              ))}
            </ul>
          )}
        </section>

        <section className="pane">
          <h3>Outcome Verification</h3>
          {contracts.length === 0 ? <div className="empty">No contracts</div> : (
            <ul className="contracts-list">
              {contracts.map((c) => (
                <li key={c.id}>
                  <div className="c-line"><strong>{c.title}</strong> — {c.state}</div>
                  <div className="c-actions"><button onClick={() => evalContract(c.id)}>Evaluate</button></div>
                </li>
              ))}
            </ul>
          )}
          {verdict && <div className="verdict">Verdict: {verdict}</div>}
        </section>
      </main>
    </div>
  );
}

export default App;
