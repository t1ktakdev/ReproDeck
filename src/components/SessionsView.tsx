import { translatedValue, useI18n } from "../i18n";
import { relativeTime, sessionMeta } from "../lib/format";
import type { Session } from "../types";

type Props = { sessions: Session[]; onNew: () => void; onOpen: (id: string) => void };

export function SessionsView({ sessions, onNew, onOpen }: Props) {
  const { t, language } = useI18n();
  return <section className="view-page root-page"><header className="view-heading app-page-heading"><div><h1>{t("sessions.title")}</h1><p>{t("sessions.description")}</p></div><button className="button primary" onClick={onNew}>{t("nav.newSession")}</button></header>
    {sessions.length === 0 ? <div className="quiet-empty"><h3>{t("home.noSessions")}</h3><p>{t("home.noSessionsHelp")}</p></div> : <div className="data-list sessions-table">{sessions.map(session => { const meta = sessionMeta(session); return <button className="data-row" key={session.id} onClick={() => onOpen(session.id)}><div className="data-row-main"><strong>{meta.title || session.id}</strong><span>{meta.actual || session.id}</span></div><span className="state-text">{translatedValue(t, "state", session.state)}</span><time>{relativeTime(session.updated_at ?? session.created_at, language)}</time></button>; })}</div>}
  </section>;
}
