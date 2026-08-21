import { DEFAULT_SETTINGS, type AppSettings } from "../types";

export type GlobalShortcut =
  | "new-session"
  | "open-project"
  | "command-palette"
  | "toggle-sidebar"
  | "toggle-inspector"
  | "settings"
  | "escape";

export type ShortcutEvent = Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "shiftKey">;

export function globalShortcut(event: ShortcutEvent): GlobalShortcut | null {
  const modifier = event.ctrlKey || event.metaKey;
  const key = event.key.toLowerCase();
  if (modifier && event.shiftKey && key === "i") return "toggle-inspector";
  if (modifier && key === "n") return "new-session";
  if (modifier && key === "o") return "open-project";
  if (modifier && key === "k") return "command-palette";
  if (modifier && key === "b") return "toggle-sidebar";
  if (modifier && event.key === ",") return "settings";
  if (!modifier && event.key === "Escape") return "escape";
  return null;
}

export type SelectKeyboardState = { open: boolean; activeIndex: number; chooseIndex: number | null };

export function selectKeyboardTransition(
  state: Pick<SelectKeyboardState, "open" | "activeIndex">,
  key: string,
  optionCount: number,
): SelectKeyboardState | null {
  if (optionCount <= 0) return null;
  if (key === "ArrowDown" || key === "ArrowUp") {
    const delta = key === "ArrowDown" ? 1 : -1;
    return {
      open: true,
      activeIndex: (state.activeIndex + delta + optionCount) % optionCount,
      chooseIndex: null,
    };
  }
  if (key === "Home" || key === "End") {
    return { open: true, activeIndex: key === "Home" ? 0 : optionCount - 1, chooseIndex: null };
  }
  if ((key === "Enter" || key === " ") && state.open) {
    return { open: false, activeIndex: state.activeIndex, chooseIndex: state.activeIndex };
  }
  if (key === "Escape" && state.open) {
    return { open: false, activeIndex: state.activeIndex, chooseIndex: null };
  }
  return null;
}

export type SettingsResetKind = "layout" | "appearance" | "ai" | "all";

export function resetSettings(settings: AppSettings, kind: SettingsResetKind): AppSettings {
  if (kind === "all") return structuredClone(DEFAULT_SETTINGS);
  if (kind === "ai") return { ...settings, ai: { ...DEFAULT_SETTINGS.ai } };
  if (kind === "appearance") {
    return {
      ...settings,
      theme: DEFAULT_SETTINGS.theme,
      ui: {
        ...settings.ui,
        density: DEFAULT_SETTINGS.ui.density,
        font_size: DEFAULT_SETTINGS.ui.font_size,
        mono_font_size: DEFAULT_SETTINGS.ui.mono_font_size,
        animations: DEFAULT_SETTINGS.ui.animations,
        reduced_motion: DEFAULT_SETTINGS.ui.reduced_motion,
        zoom: DEFAULT_SETTINGS.ui.zoom,
      },
    };
  }
  return {
    ...settings,
    ui: {
      ...settings.ui,
      sidebar_mode: DEFAULT_SETTINGS.ui.sidebar_mode,
      remember_sidebar_width: DEFAULT_SETTINGS.ui.remember_sidebar_width,
      sidebar_width: DEFAULT_SETTINGS.ui.sidebar_width,
      remember_inspector_width: DEFAULT_SETTINGS.ui.remember_inspector_width,
      inspector_width: DEFAULT_SETTINGS.ui.inspector_width,
      remember_inspector_state: DEFAULT_SETTINGS.ui.remember_inspector_state,
      inspector_open: DEFAULT_SETTINGS.ui.inspector_open,
    },
    workspace: { ...DEFAULT_SETTINGS.workspace },
  };
}

export function migrateAppSettings(value: Partial<AppSettings> | null | undefined): AppSettings {
  return {
    ...DEFAULT_SETTINGS,
    ...value,
    ui: { ...DEFAULT_SETTINGS.ui, ...value?.ui },
    behavior: { ...DEFAULT_SETTINGS.behavior, ...value?.behavior },
    workspace: { ...DEFAULT_SETTINGS.workspace, ...value?.workspace },
    ai: { ...DEFAULT_SETTINGS.ai, ...value?.ai },
  };
}

export function rememberedWorkbenchState(
  settings: AppSettings,
  current: { sidebarWidth: number; inspectorWidth: number; inspectorOpen: boolean },
) {
  return {
    sidebarWidth: settings.ui.remember_sidebar_width ? settings.ui.sidebar_width : current.sidebarWidth,
    inspectorWidth: settings.ui.remember_inspector_width ? settings.ui.inspector_width : current.inspectorWidth,
    inspectorOpen: settings.ui.remember_inspector_state ? settings.ui.inspector_open : current.inspectorOpen,
  };
}

export type EvidenceRelationship = "Supports" | "Neutral" | "Contradicts";

export function classifyEvidence(
  evidenceIds: string[],
  relationships: Readonly<Record<string, EvidenceRelationship>>,
) {
  const unique = [...new Set(evidenceIds)];
  return {
    supporting_evidence_ids: unique.filter(id => relationships[id] === "Supports"),
    neutral_evidence_ids: unique.filter(id => (relationships[id] ?? "Neutral") === "Neutral"),
    contradicting_evidence_ids: unique.filter(id => relationships[id] === "Contradicts"),
  };
}

export function shouldRevealInvestigationAfterFailure(enabled: boolean, hasFailure: boolean) {
  return enabled && hasFailure;
}

export function motionEnabled(animations: boolean, reducedMotion: boolean, systemReducedMotion: boolean) {
  return animations && !reducedMotion && !systemReducedMotion;
}

export function commandPaletteCommandIds(context: { project: boolean; session: boolean; inspector: boolean }): string[] {
  const ids = ["open-project", "new-session", "home", "projects", "sessions", "capsules", "settings", "theme"];
  if (context.project || context.session) ids.push("sidebar");
  if (context.inspector) ids.push("inspector");
  if (context.project) ids.push("run-checks", "start-investigation", "open-agent");
  return ids;
}
