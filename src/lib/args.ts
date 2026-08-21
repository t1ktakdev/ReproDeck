/** Parse a user-facing argv field into tokens. This does not invoke a shell. */
export function parseArguments(input: string): string[] {
  const result: string[] = [];
  let current = "";
  let quote: '"' | "'" | null = null;
  let escaping = false;
  let tokenStarted = false;

  for (const char of input) {
    if (escaping) { current += char; tokenStarted = true; escaping = false; continue; }
    if (char === "\\" && quote !== "'") { escaping = true; tokenStarted = true; continue; }
    if (quote) {
      if (char === quote) quote = null;
      else current += char;
      tokenStarted = true;
      continue;
    }
    if (char === '"' || char === "'") { quote = char; tokenStarted = true; continue; }
    if (/\s/.test(char)) {
      if (tokenStarted) { result.push(current); current = ""; tokenStarted = false; }
      continue;
    }
    current += char; tokenStarted = true;
  }
  if (escaping) current += "\\";
  if (quote) throw new Error("Unclosed quote in arguments.");
  if (tokenStarted) result.push(current);
  return result;
}

/** Format argv for the editable field so parsing it produces the same tokens. */
export function formatArguments(args: string[]): string {
  return args.map(value => {
    if (value.length > 0 && !/[\s"'\\]/.test(value)) return value;
    return `"${value.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
  }).join(" ");
}
