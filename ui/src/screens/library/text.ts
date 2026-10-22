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

/// The words that make a key name a secret, as whole words of the key (the core's
/// `SECRET_WORDS`).
const SECRET_WORDS = new Set([
  "TOKEN",
  "KEY",
  "SECRET",
  "PASSWORD",
  "PASSWD",
  "PWD",
  "AUTH",
  "AUTHORIZATION",
  "CREDENTIAL",
  "CREDENTIALS",
  "APIKEY",
]);

/// The last words of a key that names where or what a secret is (`NOT_SECRET_LAST_WORDS`).
const NOT_SECRET_LAST_WORDS = new Set([
  "URL", "URI", "HOST", "ENDPOINT", "ID", "NAME", "TYPE", "PATH", "FILE", "DIR", "HEADER",
  "REGION", "PORT", "ISSUER", "REALM", "SCOPE", "MODE",
]);

/// Whether a key (an env name or a header name) looks like it holds a secret (spec section 2),
/// as the core's `looks_secret` decides: one of its words, split on `_`, `-` and `.`, is a
/// secret word, and its last word is not one such as `URL`, `ID` or `HEADER`.
export function looksSecret(key: string): boolean {
  const words = key
    .toUpperCase()
    .split(/[_.-]/)
    .filter((w) => w !== "");
  const last = words[words.length - 1];
  if (last === undefined || NOT_SECRET_LAST_WORDS.has(last)) return false;
  return words.some((w) => SECRET_WORDS.has(w));
}

/// An `http(s)` URL with no user or password in it.
function plainUrl(value: string): boolean {
  const rest = /^https?:\/\/(.*)$/i.exec(value)?.[1];
  if (rest === undefined) return false;
  const authority = rest.split(/[/?#]/)[0] ?? "";
  return authority !== "" && !authority.includes("@");
}

/// A filesystem path, as the core's `looks_like_path` decides.
function looksLikePath(value: string): boolean {
  if (/^[A-Za-z]:[\\/]/.test(value) || value.startsWith("\\\\")) return true;
  if (/^(~|\.|\.\.)[\\/]/.test(value)) return true;
  return value.startsWith("/") && value.slice(1).includes("/") && !/[+= ]/.test(value);
}

/// Whether saving moves the value under `key` to Personal (the core's `hidden`): a
/// secret-looking key and a value that is not empty, not `${…}`, and not a plain URL or a path.
export function movesToVault(key: string, value: string): boolean {
  const trimmed = value.trim();
  return (
    looksSecret(key) &&
    !value.includes("${") &&
    trimmed !== "" &&
    !plainUrl(trimmed) &&
    !looksLikePath(trimmed)
  );
}
