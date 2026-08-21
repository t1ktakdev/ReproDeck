import { describe, expect, it } from "vitest";
import { DEFAULT_SETTINGS } from "../types";
import {
  classifyEvidence,
  commandPaletteCommandIds,
  globalShortcut,
  migrateAppSettings,
  motionEnabled,
  rememberedWorkbenchState,
  resetSettings,
  selectKeyboardTransition,
  shouldRevealInvestigationAfterFailure,
} from "./uiBehavior";

describe("global shortcuts", () => {
  it("maps the commands advertised by the command palette", () => {
    const event = (key: string, shiftKey = false) => ({ key, ctrlKey: true, metaKey: false, shiftKey });
    expect(globalShortcut(event("o"))).toBe("open-project");
    expect(globalShortcut(event("n"))).toBe("new-session");
    expect(globalShortcut(event("k"))).toBe("command-palette");
    expect(globalShortcut(event("b"))).toBe("toggle-sidebar");
    expect(globalShortcut(event("I", true))).toBe("toggle-inspector");
    expect(globalShortcut(event(","))).toBe("settings");
  });

  it("shows workspace commands only when their context exists", () => {
    expect(commandPaletteCommandIds({ project: false, session: false, inspector: false })).not.toContain("sidebar");
    expect(commandPaletteCommandIds({ project: true, session: false, inspector: true })).toEqual(expect.arrayContaining(["run-checks", "start-investigation", "open-agent", "sidebar", "inspector"]));
    expect(commandPaletteCommandIds({ project: false, session: true, inspector: false })).toContain("sidebar");
  });
});

describe("custom select keyboard behavior", () => {
  it("opens, wraps, commits and closes predictably", () => {
    expect(selectKeyboardTransition({ open: false, activeIndex: 0 }, "ArrowUp", 3)).toEqual({ open: true, activeIndex: 2, chooseIndex: null });
    expect(selectKeyboardTransition({ open: true, activeIndex: 2 }, "ArrowDown", 3)).toEqual({ open: true, activeIndex: 0, chooseIndex: null });
    expect(selectKeyboardTransition({ open: true, activeIndex: 1 }, "Enter", 3)?.chooseIndex).toBe(1);
    expect(selectKeyboardTransition({ open: true, activeIndex: 1 }, "Escape", 3)?.open).toBe(false);
  });
});

describe("settings semantics", () => {
  it("does not reset language with appearance", () => {
    const settings = { ...DEFAULT_SETTINGS, language: "ru" as const, theme: "dark" as const };
    const reset = resetSettings(settings, "appearance");
    expect(reset.language).toBe("ru");
    expect(reset.theme).toBe(DEFAULT_SETTINGS.theme);
  });

  it("resets AI without changing appearance or behavior", () => {
    const settings = { ...DEFAULT_SETTINGS, theme: "dark" as const, ai: { ...DEFAULT_SETTINGS.ai, enabled: true, model: "local" } };
    const reset = resetSettings(settings, "ai");
    expect(reset.ai).toEqual(DEFAULT_SETTINGS.ai);
    expect(reset.theme).toBe("dark");
    expect(reset.behavior).toEqual(settings.behavior);
  });

  it("migrates missing nested preferences from defaults", () => {
    const migrated = migrateAppSettings({ language: "ru", ui: { density: "compact" } } as Partial<typeof DEFAULT_SETTINGS>);
    expect(migrated.language).toBe("ru");
    expect(migrated.ui.density).toBe("compact");
    expect(migrated.ui.inspector_width).toBe(DEFAULT_SETTINGS.ui.inspector_width);
  });

  it("only restores widths and inspector state when their remember switches are on", () => {
    const settings = structuredClone(DEFAULT_SETTINGS);
    settings.ui.remember_sidebar_width = false;
    settings.ui.remember_inspector_width = false;
    settings.ui.remember_inspector_state = false;
    const state = rememberedWorkbenchState(settings, { sidebarWidth: 300, inspectorWidth: 600, inspectorOpen: false });
    expect(state).toEqual({ sidebarWidth: 300, inspectorWidth: 600, inspectorOpen: false });
  });
});

describe("investigation semantics", () => {
  it("classifies every unselected evidence item as neutral", () => {
    expect(classifyEvidence(["health:1", "ctx:1", "ctx:2"], { "ctx:1": "Supports", "ctx:2": "Contradicts" })).toEqual({
      supporting_evidence_ids: ["ctx:1"],
      neutral_evidence_ids: ["health:1"],
      contradicting_evidence_ids: ["ctx:2"],
    });
  });

  it("auto reveal never implies case creation", () => {
    expect(shouldRevealInvestigationAfterFailure(true, true)).toBe(true);
    expect(shouldRevealInvestigationAfterFailure(true, false)).toBe(false);
  });

  it("honors both app and system reduced-motion choices", () => {
    expect(motionEnabled(true, false, false)).toBe(true);
    expect(motionEnabled(true, false, true)).toBe(false);
    expect(motionEnabled(true, true, false)).toBe(false);
  });
});
