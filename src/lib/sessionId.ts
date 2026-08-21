const MAX_SESSION_ID_LENGTH = 96;

export function uniqueSessionId(requested: string, existingIds: Iterable<string>): string {
  const occupied = new Set(existingIds);
  const base = requested.slice(0, MAX_SESSION_ID_LENGTH);
  if (!occupied.has(base)) return base;

  for (let counter = 2; ; counter += 1) {
    const suffix = `-${counter}`;
    const candidate = `${base.slice(0, MAX_SESSION_ID_LENGTH - suffix.length)}${suffix}`;
    if (!occupied.has(candidate)) return candidate;
  }
}
