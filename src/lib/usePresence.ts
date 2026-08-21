import { useEffect, useState } from "react";

export type PresencePhase = "entering" | "open" | "closing";

export function usePresence(open: boolean, exitMs = 220): { mounted: boolean; phase: PresencePhase } {
  const [mounted, setMounted] = useState(open);
  const [phase, setPhase] = useState<PresencePhase>(open ? "open" : "closing");

  useEffect(() => {
    if (open) {
      setMounted(true);
      setPhase("entering");
      const frame = window.requestAnimationFrame(() => setPhase("open"));
      return () => window.cancelAnimationFrame(frame);
    }
    if (!mounted) return;
    setPhase("closing");
    const timeout = window.setTimeout(() => setMounted(false), exitMs);
    return () => window.clearTimeout(timeout);
  }, [exitMs, mounted, open]);

  return { mounted, phase };
}
