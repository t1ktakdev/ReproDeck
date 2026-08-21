import { useEffect, useMemo, useState } from "react";
import { useI18n } from "../i18n";
import { bridgeMessage, confirmAction, invokeTauri, revealLocalPath } from "../lib/tauri";
import { resetSettings, type SettingsResetKind } from "../lib/uiBehavior";
import type { AiConnectionStatus, AppSettings, GitHubStatus } from "../types";
import { SegmentedControl, Select, SettingRow, Toggle } from "./ui";

type Props = { settings: AppSettings; onSave: (settings: AppSettings) => Promise<void> };
type SettingsCategory = "appearance" | "behavior" | "ai" | "privacy" | "advanced";

export function SettingsView({ settings, onSave }: Props) {
  const { t } = useI18n();
  const [category, setCategory] = useState<SettingsCategory>("appearance");
  const [search, setSearch] = useState("");
  const [draft, setDraft] = useState(settings);
  const [github, setGithub] = useState<GitHubStatus | null>(null);
  const [storagePath, setStoragePath] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [aiStatus, setAiStatus] = useState<AiConnectionStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const dirty = useMemo(() => JSON.stringify(draft) !== JSON.stringify(settings), [draft, settings]);

  useEffect(() => setDraft(settings), [settings]);
  useEffect(() => { if (dirty && notice) setNotice(null); }, [dirty, notice]);
  useEffect(() => {
    void Promise.all([
      invokeTauri<GitHubStatus>("github_status").then(setGithub),
      invokeTauri<string>("storage_location").then(setStoragePath),
    ]).catch(nextError => setError(bridgeMessage(nextError)));
  }, []);

  async function save(value = draft) {
    setBusy("save"); setError(null); setNotice(null);
    try {
      await onSave(value);
      setNotice(t("settings.saved"));
    } catch (nextError) { setError(bridgeMessage(nextError)); }
    finally { setBusy(null); }
  }

  async function refreshGitHub() {
    setError(null);
    try { setGithub(await invokeTauri<GitHubStatus>("github_status")); }
    catch (nextError) { setError(bridgeMessage(nextError)); }
  }

  async function testAi() {
    if (!await confirmAction(t("confirm.aiNetwork"))) return;
    setBusy("ai"); setError(null); setAiStatus(null);
    try {
      setAiStatus(await invokeTauri<AiConnectionStatus>("ai_test_connection", {
        baseUrl: draft.ai.base_url,
        model: draft.ai.model,
        apiKey: apiKey || null,
        timeoutSecs: draft.ai.timeout_secs,
        maxTokens: draft.ai.max_tokens,
        temperature: draft.ai.temperature,
        confirmedNetwork: true,
      }));
    } catch (nextError) { setError(bridgeMessage(nextError)); }
    finally { setBusy(null); }
  }

  async function reset(kind: SettingsResetKind) {
    if (!await confirmAction(t(`settings.resetConfirm.${kind}`))) return;
    const next = resetSettings(draft, kind);
    setDraft(next);
    await save(next);
  }

  const categories: { id: SettingsCategory; label: string; help: string }[] = [
    { id: "appearance", label: t("settings.appearance"), help: t("settings.appearanceHelp") },
    { id: "behavior", label: t("settings.behavior"), help: t("settings.behaviorHelp") },
    { id: "ai", label: t("settings.ai"), help: t("settings.aiCategoryHelp") },
    { id: "privacy", label: t("settings.privacy"), help: t("settings.privacyCategoryHelp") },
    { id: "advanced", label: t("settings.advanced"), help: t("settings.advancedHelp") },
  ];
  const searchableSettings = [
    ["appearance", "settings.theme", "settings.themeHelp"], ["appearance", "settings.language", "settings.languageHelp"], ["appearance", "settings.uiDensity", "settings.uiDensityHelp"], ["appearance", "settings.uiFontSize", "settings.uiFontSizeHelp"], ["appearance", "settings.monoFontSize", "settings.monoFontSizeHelp"], ["appearance", "settings.zoom", "settings.zoomHelp"], ["appearance", "settings.animations", "settings.animationsHelp"], ["appearance", "settings.reducedMotion", "settings.reducedMotionHelp"], ["appearance", "settings.sidebarMode", "settings.sidebarModeHelp"], ["appearance", "settings.rememberSidebar", "settings.rememberSidebarHelp"], ["appearance", "settings.rememberInspectorWidth", "settings.rememberInspectorWidthHelp"], ["appearance", "settings.rememberInspector", "settings.rememberInspectorHelp"],
    ["behavior", "settings.restoreLastProject", "settings.restoreLastProjectHelp"], ["behavior", "settings.restoreWorkspace", "settings.restoreWorkspaceHelp"], ["behavior", "settings.autoInvestigation", "settings.autoInvestigationHelp"], ["behavior", "settings.autoScrollLogs", "settings.autoScrollLogsHelp"], ["behavior", "settings.openLogsFailure", "settings.openLogsFailureHelp"], ["behavior", "settings.notifications", "settings.notificationsHelp"],
    ["ai", "settings.aiEnabled", "settings.aiEnabledHelp"], ["ai", "settings.provider", "settings.providerHelp"], ["ai", "settings.baseUrl", "settings.baseUrlHelp"], ["ai", "settings.model", "settings.modelHelp"], ["ai", "settings.timeout", "settings.timeoutHelp"], ["ai", "settings.temperature", "settings.temperatureHelp"],
    ["privacy", "settings.secretPolicy", "settings.secretPolicyHelp"], ["privacy", "settings.apiKeyPolicy", "settings.apiKeyPolicyHelp"], ["privacy", "settings.telemetry", "settings.telemetryHelp"], ["privacy", "settings.originalProtection", "settings.originalProtectionHelp"],
    ["advanced", "settings.storageLocation", "settings.storageLocationHelp"], ["advanced", "settings.resetLayout", "settings.resetLayoutHelp"], ["advanced", "settings.resetAppearance", "settings.resetAppearanceHelp"], ["advanced", "settings.resetAi", "settings.resetAiHelp"],
  ] as const;
  const searchResults = useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    if (!query) return [];
    return searchableSettings.filter(([, label, help]) => `${t(label)} ${t(help)}`.toLocaleLowerCase().includes(query));
  }, [search, t]);

  return <section className="settings-workbench">
    <header className="settings-header">
      <div><h1>{t("settings.title")}</h1><p>{t("settings.description")}</p><label className="settings-search"><span className="sr-only">{t("settings.search")}</span><input value={search} onChange={event => setSearch(event.target.value)} placeholder={t("settings.search")}/>{search && <button aria-label={t("common.clear")} onClick={() => setSearch("")}>×</button>}</label></div>
      <div className="settings-save-area">{dirty ? <span className="unsaved-indicator"><i/>{t("settings.unsaved")}</span> : notice ? <span className="saved-indicator"><i/>{notice}</span> : null}<button className="button primary" disabled={busy !== null || !dirty} onClick={() => void save()}>{busy === "save" ? t("common.loading") : t("settings.save")}</button></div>
    </header>

    <div className="settings-layout">
      <nav className="settings-nav" aria-label={t("settings.title")}>{categories.map(item => <button key={item.id} className={category === item.id ? "active" : ""} onClick={() => setCategory(item.id)}><strong>{item.label}</strong><small>{item.help}</small></button>)}</nav>
      <main className="settings-content">
        {search.trim() && <section className="settings-search-results"><header className="preference-heading"><h2>{t("settings.searchResults")}</h2><p>{t("settings.searchResultsHelp").replace("{count}", String(searchResults.length))}</p></header>{searchResults.map(([nextCategory, label, help]) => <button key={`${nextCategory}:${label}`} onClick={() => { setCategory(nextCategory); setSearch(""); }}><span><strong>{t(label)}</strong><small>{t(help)}</small></span><b>{t(`settings.${nextCategory}`)}</b></button>)}{searchResults.length === 0 && <div className="plain-empty"><strong>{t("palette.noResults")}</strong></div>}</section>}
        {!search.trim() && <>
        {category === "appearance" && <>
          <header className="preference-heading"><h2>{t("settings.appearance")}</h2><p>{t("settings.appearanceIntro")}</p></header>
          <section className="preference-section"><h3>{t("settings.colorAndType")}</h3>
            <SettingRow label={t("settings.theme")} description={t("settings.themeHelp")}><SegmentedControl ariaLabel={t("settings.theme")} value={draft.theme} options={[{ value: "system", label: t("settings.system") }, { value: "dark", label: t("settings.dark") }, { value: "light", label: t("settings.light") }]} onChange={theme => setDraft({ ...draft, theme })}/></SettingRow>
            <SettingRow label={t("settings.language")} description={t("settings.languageHelp")}><Select ariaLabel={t("settings.language")} value={draft.language} options={[{ value: "en", label: t("settings.english") }, { value: "ru", label: t("settings.russian") }]} onChange={language => setDraft({ ...draft, language })}/></SettingRow>
            <SettingRow label={t("settings.uiDensity")} description={t("settings.uiDensityHelp")}><Select ariaLabel={t("settings.uiDensity")} value={draft.ui.density} options={[{ value: "comfortable", label: t("settings.comfortable") }, { value: "compact", label: t("settings.compact") }]} onChange={density => setDraft({ ...draft, ui: { ...draft.ui, density } })}/></SettingRow>
            <SettingRow label={t("settings.uiFontSize")} description={t("settings.uiFontSizeHelp")}><Select ariaLabel={t("settings.uiFontSize")} value={draft.ui.font_size} options={[{ value: "small", label: t("settings.small") }, { value: "default", label: t("settings.default") }, { value: "large", label: t("settings.large") }]} onChange={font_size => setDraft({ ...draft, ui: { ...draft.ui, font_size } })}/></SettingRow>
            <SettingRow label={t("settings.monoFontSize")} description={t("settings.monoFontSizeHelp")}><Select ariaLabel={t("settings.monoFontSize")} value={draft.ui.mono_font_size} options={[12,13,14,15].map(value => ({ value: value as 12|13|14|15, label: `${value} px` }))} onChange={mono_font_size => setDraft({ ...draft, ui: { ...draft.ui, mono_font_size } })}/></SettingRow>
            <SettingRow label={t("settings.zoom")} description={t("settings.zoomHelp")}><Select ariaLabel={t("settings.zoom")} value={draft.ui.zoom} options={[90,100,110,125].map(value => ({ value: value as 90|100|110|125, label: `${value}%` }))} onChange={zoom => setDraft({ ...draft, ui: { ...draft.ui, zoom } })}/></SettingRow>
          </section>
          <section className="preference-section"><h3>{t("settings.motion")}</h3>
            <SettingRow label={t("settings.animations")} description={t("settings.animationsHelp")}><Toggle label={t("settings.animations")} checked={draft.ui.animations} onChange={animations => setDraft({ ...draft, ui: { ...draft.ui, animations } })}/></SettingRow>
            <SettingRow label={t("settings.reducedMotion")} description={t("settings.reducedMotionHelp")}><Toggle label={t("settings.reducedMotion")} checked={draft.ui.reduced_motion} onChange={reduced_motion => setDraft({ ...draft, ui: { ...draft.ui, reduced_motion } })}/></SettingRow>
          </section>
          <section className="preference-section"><h3>{t("settings.workbench")}</h3>
            <SettingRow label={t("settings.sidebarMode")} description={t("settings.sidebarModeHelp")}><Select ariaLabel={t("settings.sidebarMode")} value={draft.ui.sidebar_mode} options={[{ value: "expanded", label: t("settings.expanded") }, { value: "compact", label: t("settings.compact") }]} onChange={sidebar_mode => setDraft({ ...draft, ui: { ...draft.ui, sidebar_mode } })}/></SettingRow>
            <SettingRow label={t("settings.rememberSidebar")} description={t("settings.rememberSidebarHelp")}><Toggle label={t("settings.rememberSidebar")} checked={draft.ui.remember_sidebar_width} onChange={remember_sidebar_width => setDraft({ ...draft, ui: { ...draft.ui, remember_sidebar_width } })}/></SettingRow>
            <SettingRow label={t("settings.sidebarWidth")} description={t("settings.sidebarWidthHelp")}><input className="number-input" type="number" min={220} max={340} value={draft.ui.sidebar_width} onChange={event => setDraft({ ...draft, ui: { ...draft.ui, sidebar_width: Math.max(220, Math.min(340, Number(event.target.value) || 256)) } })}/></SettingRow>
            <SettingRow label={t("settings.inspectorWidth")} description={t("settings.inspectorWidthHelp")}><input className="number-input" type="number" min={360} max={760} value={draft.ui.inspector_width} onChange={event => setDraft({ ...draft, ui: { ...draft.ui, inspector_width: Math.max(360, Math.min(760, Number(event.target.value) || 480)) } })}/></SettingRow>
            <SettingRow label={t("settings.rememberInspectorWidth")} description={t("settings.rememberInspectorWidthHelp")}><Toggle label={t("settings.rememberInspectorWidth")} checked={draft.ui.remember_inspector_width} onChange={remember_inspector_width => setDraft({ ...draft, ui: { ...draft.ui, remember_inspector_width } })}/></SettingRow>
            <SettingRow label={t("settings.rememberInspector")} description={t("settings.rememberInspectorHelp")}><Toggle label={t("settings.rememberInspector")} checked={draft.ui.remember_inspector_state} onChange={remember_inspector_state => setDraft({ ...draft, ui: { ...draft.ui, remember_inspector_state } })}/></SettingRow>
          </section>
        </>}

        {category === "privacy" && <>
          <header className="preference-heading"><h2>{t("settings.privacy")}</h2><p>{t("settings.privacyIntro")}</p></header>
          <section className="preference-section"><h3>{t("settings.enforcedProtections")}</h3>
            <SettingRow label={t("settings.secretPolicy")} description={t("settings.secretPolicyHelp")}><span className="locked-setting">{t("settings.alwaysOn")}</span></SettingRow>
            <SettingRow label={t("settings.apiKeyPolicy")} description={t("settings.apiKeyPolicyHelp")}><span className="locked-setting">{t("settings.memoryOnly")}</span></SettingRow>
            <SettingRow label={t("settings.originalProtection")} description={t("settings.originalProtectionHelp")}><span className="locked-setting">{t("settings.alwaysOn")}</span></SettingRow>
            <SettingRow label={t("settings.telemetry")} description={t("settings.telemetryHelp")}><span className="locked-setting">{t("settings.off")}</span></SettingRow>
          </section>
        </>}

        {category === "behavior" && <>
          <header className="preference-heading"><h2>{t("settings.behavior")}</h2><p>{t("settings.behaviorIntro")}</p></header>
          <section className="preference-section"><h3>{t("settings.startup")}</h3>
            <SettingRow label={t("settings.restoreLastProject")} description={t("settings.restoreLastProjectHelp")}><Toggle label={t("settings.restoreLastProject")} checked={draft.behavior.restore_last_project} onChange={restore_last_project => setDraft({ ...draft, behavior: { ...draft.behavior, restore_last_project } })}/></SettingRow>
            <SettingRow label={t("settings.restoreWorkspace")} description={t("settings.restoreWorkspaceHelp")}><Toggle label={t("settings.restoreWorkspace")} checked={draft.behavior.restore_last_workspace} onChange={restore_last_workspace => setDraft({ ...draft, behavior: { ...draft.behavior, restore_last_workspace } })}/></SettingRow>
          </section>
          <section className="preference-section"><h3>{t("settings.checksAndLogs")}</h3>
            <SettingRow label={t("settings.autoInvestigation")} description={t("settings.autoInvestigationHelp")}><Toggle label={t("settings.autoInvestigation")} checked={draft.behavior.auto_open_investigation} onChange={auto_open_investigation => setDraft({ ...draft, behavior: { ...draft.behavior, auto_open_investigation } })}/></SettingRow>
            <SettingRow label={t("settings.autoScrollLogs")} description={t("settings.autoScrollLogsHelp")}><Toggle label={t("settings.autoScrollLogs")} checked={draft.behavior.auto_scroll_logs} onChange={auto_scroll_logs => setDraft({ ...draft, behavior: { ...draft.behavior, auto_scroll_logs } })}/></SettingRow>
            <SettingRow label={t("settings.openLogsFailure")} description={t("settings.openLogsFailureHelp")}><Toggle label={t("settings.openLogsFailure")} checked={draft.behavior.open_logs_on_failure} onChange={open_logs_on_failure => setDraft({ ...draft, behavior: { ...draft.behavior, open_logs_on_failure } })}/></SettingRow>
            <SettingRow label={t("settings.notifications")} description={t("settings.notificationsHelp")}><Toggle label={t("settings.notifications")} checked={draft.behavior.notifications} onChange={notifications => setDraft({ ...draft, behavior: { ...draft.behavior, notifications } })}/></SettingRow>
          </section>
        </>}

        {category === "ai" && <>
          <header className="preference-heading"><h2>{t("settings.ai")}</h2><p>{t("settings.aiHelp")}</p></header>
          <section className="preference-section"><h3>{t("settings.connection")}</h3>
            <SettingRow label={t("settings.aiEnabled")} description={t("settings.aiEnabledHelp")}><Toggle label={t("settings.aiEnabled")} checked={draft.ai.enabled} onChange={enabled => setDraft({ ...draft, ai: { ...draft.ai, enabled } })}/></SettingRow>
            <SettingRow label={t("settings.provider")} description={t("settings.providerHelp")}><Select ariaLabel={t("settings.provider")} value={draft.ai.provider} options={[{ value: "openai-compatible", label: "OpenAI-compatible" }]} onChange={provider => setDraft({ ...draft, ai: { ...draft.ai, provider } })}/></SettingRow>
            <SettingRow label={t("settings.baseUrl")} description={t("settings.baseUrlHelp")}><input value={draft.ai.base_url} onChange={event => setDraft({ ...draft, ai: { ...draft.ai, base_url: event.target.value } })}/></SettingRow>
            <SettingRow label={t("settings.model")} description={t("settings.modelHelp")}><input value={draft.ai.model} onChange={event => setDraft({ ...draft, ai: { ...draft.ai, model: event.target.value } })}/></SettingRow>
            <div className="preset-row"><span>{t("settings.localEndpoints")}</span><button className="button small" type="button" onClick={() => setDraft({ ...draft, ai: { ...draft.ai, enabled: true, base_url: "http://127.0.0.1:1234/v1", model: "reprodeck-local" } })}>LM Studio</button><button className="button small" type="button" onClick={() => setDraft({ ...draft, ai: { ...draft.ai, enabled: true, base_url: "http://127.0.0.1:11434/v1" } })}>Ollama</button></div>
          </section>
          <section className="preference-section"><h3>{t("settings.request")}</h3>
            <SettingRow label={t("settings.timeout")} description={t("settings.timeoutHelp")}><input className="number-input" type="number" min={5} max={300} value={draft.ai.timeout_secs} onChange={event => setDraft({ ...draft, ai: { ...draft.ai, timeout_secs: Math.max(5, Math.min(300, Number(event.target.value) || 60)) } })}/></SettingRow>
            <SettingRow label={t("settings.maxTokens")} description={t("settings.maxTokensHelp")}><input className="number-input" type="number" min={128} max={32768} value={draft.ai.max_tokens} onChange={event => setDraft({ ...draft, ai: { ...draft.ai, max_tokens: Math.max(128, Math.min(32768, Number(event.target.value) || 2048)) } })}/></SettingRow>
            <SettingRow label={t("settings.temperature")} description={t("settings.temperatureHelp")}><input className="number-input" type="number" min={0} max={2} step={0.1} value={draft.ai.temperature} onChange={event => setDraft({ ...draft, ai: { ...draft.ai, temperature: Math.max(0, Math.min(2, Number(event.target.value) || 0)) } })}/></SettingRow>
          </section>
          <section className="preference-section"><h3>{t("settings.connectionTest")}</h3>
            <SettingRow label={t("settings.apiKey")} description={t("settings.apiKeyEphemeral")}><input type="password" autoComplete="off" value={apiKey} onChange={event => setApiKey(event.target.value)}/></SettingRow>
            <div className="connection-action"><button className="button" disabled={busy !== null || !draft.ai.model.trim()} onClick={() => void testAi()}>{busy === "ai" ? t("common.loading") : t("settings.test")}</button>{aiStatus && <span className="connection-result"><i/>{aiStatus.provider} · {aiStatus.model}</span>}</div>
            <div className="privacy-lock"><strong>{t("settings.privacyTitle")}</strong><p>{t("settings.privacyHelp")}</p></div>
          </section>
        </>}

        {category === "advanced" && <>
          <header className="preference-heading"><h2>{t("settings.advanced")}</h2><p>{t("settings.advancedIntro")}</p></header>
          <section className="preference-section"><h3>{t("settings.storage")}</h3>
            <SettingRow label={t("settings.storageLocation")} description={t("settings.storageLocationHelp")}><button className="path-control" title={storagePath} disabled={!storagePath} onClick={() => void revealLocalPath(storagePath).catch(nextError => setError(bridgeMessage(nextError)))}>{storagePath || "—"}</button></SettingRow>
          </section>
          <section className="preference-section"><h3>{t("settings.github")}</h3>
            <SettingRow label={t("settings.githubStatus")} description={t("settings.githubHelp")}><div className="github-status"><span>{github?.installed ? t("status.installed") : t("status.notInstalled")}</span><small>{github?.authenticated ? t("status.authenticated") : t("status.notAuthenticated")}</small><button className="button small" onClick={() => void refreshGitHub()}>{t("settings.refresh")}</button></div></SettingRow>
          </section>
          <section className="preference-section reset-section"><h3>{t("settings.reset")}</h3>
            <div className="reset-row"><div><strong>{t("settings.resetLayout")}</strong><p>{t("settings.resetLayoutHelp")}</p></div><button className="button" onClick={() => void reset("layout")}>{t("settings.resetAction")}</button></div>
            <div className="reset-row"><div><strong>{t("settings.resetAppearance")}</strong><p>{t("settings.resetAppearanceHelp")}</p></div><button className="button" onClick={() => void reset("appearance")}>{t("settings.resetAction")}</button></div>
            <div className="reset-row"><div><strong>{t("settings.resetAi")}</strong><p>{t("settings.resetAiHelp")}</p></div><button className="button" onClick={() => void reset("ai")}>{t("settings.resetAction")}</button></div>
            <div className="reset-row danger-row"><div><strong>{t("settings.resetAll")}</strong><p>{t("settings.resetAllHelp")}</p></div><button className="button danger" onClick={() => void reset("all")}>{t("settings.resetAction")}</button></div>
          </section>
        </>}
        </>}
        {error && <div className="inline-error page-message">{error}</div>}
      </main>
    </div>
  </section>;
}
