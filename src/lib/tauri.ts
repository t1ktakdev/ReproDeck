import type { BridgeError } from "../types";

declare global {
  interface Window {
    __TAURI_INTERNALS__?: { invoke?: unknown };
  }
}

export class TauriUnavailableError extends Error {
  constructor() {
    super("ReproDeck is running in browser preview. Open it with `npm run tauri dev` to use local project features.");
    this.name = "TauriUnavailableError";
  }
}

export function hasTauriRuntime(): boolean {
  return typeof window !== "undefined" && typeof window.__TAURI_INTERNALS__?.invoke === "function";
}

export async function invokeTauri<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!hasTauriRuntime()) throw new TauriUnavailableError();
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(command, args);
}

export function bridgeMessage(error: unknown): string {
  if (error instanceof TauriUnavailableError) return error.message;
  if (typeof error === "string") return error;
  if (error && typeof error === "object") {
    const value = error as BridgeError;
    if (value.message) return value.message;
  }
  if (error instanceof Error && error.message) return error.message;
  return "ReproDeck could not complete that operation.";
}

export function bridgeCode(error: unknown): string | undefined {
  if (error && typeof error === "object") return (error as BridgeError).code;
  return undefined;
}

export async function confirmAction(message: string): Promise<boolean> {
  if (!hasTauriRuntime()) return window.confirm(message);
  const { confirm } = await import("@tauri-apps/plugin-dialog");
  const russian = document.documentElement.lang.toLowerCase().startsWith("ru");
  return confirm(message, {
    title: "ReproDeck",
    kind: "warning",
    okLabel: russian ? "Продолжить" : "Continue",
    cancelLabel: russian ? "Отмена" : "Cancel",
  });
}


export async function chooseRepositoryDirectory(title = "Choose Git repository"): Promise<string | null> {
  if (!hasTauriRuntime()) throw new TauriUnavailableError();
  const { open } = await import("@tauri-apps/plugin-dialog");
  const selected = await open({ directory: true, multiple: false, title });
  return typeof selected === "string" ? selected : null;
}

export async function revealLocalPath(path: string): Promise<void> {
  if (!hasTauriRuntime()) throw new TauriUnavailableError();
  const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
  await revealItemInDir(path);
}

export async function chooseCapsuleFile(title = "Import ReproDeck capsule", filterName = "ReproDeck capsule"): Promise<string | null> {
  if (!hasTauriRuntime()) throw new TauriUnavailableError();
  const { open } = await import("@tauri-apps/plugin-dialog");
  const selected = await open({
    multiple: false,
    directory: false,
    title,
    filters: [{ name: filterName, extensions: ["reprodeck"] }],
  });
  return typeof selected === "string" ? selected : null;
}

export async function chooseCapsuleDestination(defaultName: string, title = "Export ReproDeck capsule", filterName = "ReproDeck capsule"): Promise<string | null> {
  if (!hasTauriRuntime()) throw new TauriUnavailableError();
  const { save } = await import("@tauri-apps/plugin-dialog");
  const selected = await save({
    title,
    defaultPath: defaultName.endsWith(".reprodeck") ? defaultName : `${defaultName}.reprodeck`,
    filters: [{ name: filterName, extensions: ["reprodeck"] }],
  });
  return typeof selected === "string" ? selected : null;
}

export async function openExternalUrl(url: string): Promise<void> {
  if (!hasTauriRuntime()) throw new TauriUnavailableError();
  if (!/^https:\/\//i.test(url)) throw new Error("Only HTTPS URLs can be opened from ReproDeck.");
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  await openUrl(url);
}
