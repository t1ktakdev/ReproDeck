import type { Session, SessionMeta } from "../types";

export function sessionMeta(session: Session | null): SessionMeta {
  if (!session?.meta) return { title: "", expected: "", actual: "", notes: "" };
  try {
    const parsed = JSON.parse(session.meta) as Partial<SessionMeta>;
    return {
      title: typeof parsed.title === "string" ? parsed.title : "",
      expected: typeof parsed.expected === "string" ? parsed.expected : "",
      actual: typeof parsed.actual === "string" ? parsed.actual : "",
      notes: typeof parsed.notes === "string" ? parsed.notes : "",
    };
  } catch {
    return { title: "", expected: "", actual: "", notes: "" };
  }
}

export function relativeTime(seconds: number, locale?: string): string {
  const delta = Math.max(0, Math.floor(Date.now() / 1000 - seconds));
  const formatter = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  if (delta < 60) return formatter.format(-delta, "second");
  if (delta < 3600) return formatter.format(-Math.floor(delta / 60), "minute");
  if (delta < 86400) return formatter.format(-Math.floor(delta / 3600), "hour");
  return formatter.format(-Math.floor(delta / 86400), "day");
}

export function clockTime(seconds: number): string {
  return new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit" }).format(new Date(seconds * 1000));
}

export function duration(milliseconds: number | null): string {
  if (milliseconds === null) return "—";
  if (milliseconds < 1000) return `${milliseconds} ms`;
  return `${(milliseconds / 1000).toFixed(milliseconds < 10_000 ? 2 : 1)} s`;
}

export function bytes(value: number): string {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MB`;
}

export function repoName(path: string): string {
  const normalized = path.replace(/\\/g, "/").replace(/\/+$/, "");
  return normalized.split("/").pop() || normalized;
}

export function commandText(executable: string, args: string[]): string {
  const quote = (value: string) => /\s|"/.test(value) ? `"${value.replace(/"/g, '\\"')}"` : value;
  return [executable, ...args].map(quote).join(" ");
}
