import type { GithubEvent, Run, Watcher, WatcherConfig } from "@/api/types";

/// How a watcher's GitHub events read in its summary.
const GITHUB_WORDS: Record<GithubEvent, string> = {
  issue: "issues",
  pull_request: "PRs",
  comment: "comments",
  push: "pushes",
  ci: "CI runs",
};

/// A long path as its root, an ellipsis and its last part: `C:\…\Invoices`.
export function shortPath(path: string): string {
  if (path.length <= 40) return path;
  const sep = path.includes("\\") ? "\\" : "/";
  const parts = path.split(/[\\/]/).filter(Boolean);
  if (parts.length < 2) return path;
  return `${parts[0]}${sep}…${sep}${parts[parts.length - 1]}`;
}

/// What a watcher watches, in one line (watchers spec section 8).
export function watcherSummary(w: Watcher): string {
  const c = w.config;
  switch (c.kind) {
    case "folder":
      return `${c.pattern || "Every file"} in ${shortPath(c.path)}${c.recursive ? ", with subfolders" : ""}`;
    case "email": {
      const parts = [c.from ?? c.address];
      if (c.subject) parts.push(`subject '${c.subject}'`);
      if (c.has_attachment === true) parts.push("with attachments");
      if (c.has_attachment === false) parts.push("without attachments");
      parts.push(`Gmail label ${c.label}`);
      return parts.join(" · ");
    }
    case "github":
      return `${c.repos.join(", ")} · ${c.events.map((e) => GITHUB_WORDS[e]).join(", ")}`;
  }
}

/// Whether the watcher is on, paused by the person, or paused by Dots and why.
export function watcherState(w: Watcher): string {
  if (w.paused_reason === "flood") return "paused: too many runs";
  if (w.paused_reason === "error") return `paused: ${w.last_error ?? "an error"}`;
  return w.enabled ? "on" : "paused by you";
}

const text = (v: unknown) => (typeof v === "string" ? v : "");

/// A watcher run's event in a line, as its run card shows it; `null` for a payload it cannot
/// read.
export function watcherEventLine(payload: unknown): string | null {
  if (typeof payload !== "object" || payload === null) return null;
  const p = payload as Record<string, unknown>;
  const n = typeof p.number === "number" ? p.number : "";
  switch (p.source) {
    case "folder": {
      const what = { created: "New file", changed: "Changed file", deleted: "Deleted file" }[
        text(p.event)
      ];
      return what ? `${what}: ${text(p.relative)}` : null;
    }
    case "email":
      return `Email from ${text(p.from)}: ${text(p.subject)}`;
    case "github":
      switch (p.event) {
        case "issue":
          return `New issue #${n}: ${text(p.title)}`;
        case "pull_request":
          return `New PR #${n}: ${text(p.title)}`;
        case "comment":
          return `New comment on #${n}: ${text(p.title)}`;
        case "push":
          return `Push to ${text(p.branch)}: ${text(p.title)}`;
        case "ci":
          return `CI ${text(p.conclusion)} on ${text(p.branch)}: ${text(p.title)}`;
        default:
          return null;
      }
    default:
      return null;
  }
}

/// "<watcher name> · <event line>" for a watcher run; `null` for any other run.
export function watcherTitle(run: Run): string | null {
  if (run.trigger !== "watcher") return null;
  const line = watcherEventLine(run.payload);
  if (line === null) return null;
  const name = text((run.payload as Record<string, unknown>).watcher_name);
  return name ? `${name} · ${line}` : line;
}

/// The name a new watcher gets until the person writes one.
export function defaultWatcherName(config: WatcherConfig): string {
  switch (config.kind) {
    case "folder": {
      const last = config.path.split(/[\\/]/).filter(Boolean).pop();
      return last ? `${last} folder` : "Folder";
    }
    case "email":
      return config.address ? `${config.address} mail` : "Email";
    case "github":
      return config.repos[0] ?? "GitHub";
  }
}

/// A path for comparing: `/` separators, lower case, no trailing separator.
function comparable(path: string): string {
  return path.trim().replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
}

/// Whether `folder` is `workdir` or below it, `\` and `/` alike, in any case.
export function folderInside(folder: string, workdir: string): boolean {
  const f = comparable(folder);
  const w = comparable(workdir);
  return f === w || f.startsWith(`${w}/`);
}
