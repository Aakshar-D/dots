/// One `KEY=VALUE` (or `Name: value`) per line, as the server form takes env and headers.
/// Blank lines are skipped. `line` of a failure counts from 1, blank lines included.
export type Lines = { ok: true; entries: Record<string, string> } | { ok: false; line: number };

export function parseLines(text: string, separator: "=" | ":"): Lines {
  const entries: Record<string, string> = {};
  const lines = text.split(/\r?\n/);
  for (const [index, raw] of lines.entries()) {
    if (raw.trim() === "") continue;
    const at = raw.indexOf(separator);
    const key = at < 0 ? "" : raw.slice(0, at).trim();
    if (key === "") return { ok: false, line: index + 1 };
    entries[key] = raw.slice(at + 1).trim();
  }
  return { ok: true, entries };
}

/// The inverse of `parseLines`.
export function joinLines(entries: Record<string, string>, separator: "=" | ":"): string {
  const glue = separator === "=" ? "=" : ": ";
  return Object.entries(entries)
    .map(([k, v]) => `${k}${glue}${v}`)
    .join("\n");
}

/// The names of the `${VAR}` references in `texts`, once each, in the order they appear.
export function variablesIn(texts: string[]): string[] {
  const names = new Set<string>();
  for (const text of texts) {
    for (const match of text.matchAll(/\$\{([A-Za-z_][A-Za-z0-9_]*)\}/g)) names.add(match[1]!);
  }
  return [...names];
}

/// Whether a key (an env name or a header name) looks like it holds a secret (spec section 2).
export function looksSecret(key: string): boolean {
  return /token|key|secret|password|auth/i.test(key);
}
