import type { ConnectorEntry, ConnectorState, KeyField } from "@/api/types";

/// A field's value as the core stores it: spaces taken out of a Google app password, else
/// trimmed.
export function cleaned(field: KeyField, raw: string): string {
  return field.strip_spaces ? raw.replace(/\s+/g, "") : raw.trim();
}

/// What is wrong with a typed value, in the core's words (`connectors::check_field`), or null.
/// The core checks again; this only spares a dialog for a value it would refuse.
export function fieldError(field: KeyField, raw: string): string | null {
  const value = cleaned(field, raw);
  if (value === "") return "Enter a value.";
  const format = field.format;
  switch (format.kind) {
    case "any":
      return null;
    case "prefix":
      return format.prefixes.some((p) => value.startsWith(p))
        ? null
        : `It should start with ${format.prefixes.join(" or ")}.`;
    case "email": {
      const at = value.indexOf("@");
      return at > 0 && value.slice(at + 1).includes(".") && !value.includes(" ")
        ? null
        : "Enter an email address.";
    }
    case "letters":
      return value.length === format.length && /^[A-Za-z]+$/.test(value)
        ? null
        : `It should be ${format.length} letters.`;
  }
}

/// The status line of a card, as words: the screen never says it by colour alone.
export const STATE: Record<ConnectorState, { label: string; className: string }> = {
  not_connected: { label: "Not connected", className: "bg-muted text-muted-foreground" },
  connected: { label: "Connected", className: "border-success/40 bg-transparent text-success" },
  needs_sign_in: {
    label: "Needs sign-in again",
    className: "border-needs-border bg-transparent text-needs",
  },
  error: { label: "Error", className: "border-destructive/40 bg-transparent text-destructive" },
  name_taken: {
    label: "Name in use",
    className: "border-needs-border bg-transparent text-needs",
  },
  coming_soon: { label: "Coming soon", className: "bg-muted text-muted-foreground" },
};

export const CATEGORY: Record<ConnectorEntry["category"], string> = {
  communication: "Email and chat",
  work: "Work and projects",
  developer: "Developer tools",
  search: "Search",
  files: "Files",
  business: "Business",
};

/// Who makes a connector's server: "by GitHub · official".
export function makerLabel(c: ConnectorEntry): string {
  return `by ${c.maker} · ${c.trust === "official" ? "official" : "community"}`;
}

/// Whether the connector has a Library server now (connected, needing sign-in, or failing).
export function isConnected(c: ConnectorEntry): boolean {
  return c.server_id !== null;
}
