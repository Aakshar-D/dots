import type { SearchHit, SearchKind } from "@/api/types";

/// Where a matched word starts in a snippet (the core's `search::HIT_START`).
export const HIT_START = "";
/// Where a matched word ends in a snippet (the core's `search::HIT_END`).
export const HIT_END = "";

/// One piece of a snippet: plain text, or a word that matched.
export type SnippetPart = { text: string; hit: boolean };

/// `snippet` cut at its marks into plain text and matched words, the marks left out. The text
/// is shown as text, never as markup; a mark with no end runs to the end of the snippet.
export function snippetParts(snippet: string): SnippetPart[] {
  const parts: SnippetPart[] = [];
  let hit = false;
  let text = "";
  for (const ch of snippet) {
    if (ch === HIT_START || ch === HIT_END) {
      if (text) parts.push({ text, hit });
      text = "";
      hit = ch === HIT_START;
    } else {
      text += ch;
    }
  }
  if (text) parts.push({ text, hit });
  return parts;
}

/// The address a hit opens.
export function hitPath(hit: SearchHit): string {
  const e = encodeURIComponent;
  const t = hit.target;
  switch (t.type) {
    case "topic":
      return `/home/${e(t.dot_id)}/${e(t.root_run_id)}`;
    case "run":
      return `/runs/${e(t.run_id)}`;
    case "dot":
      return `/home/${e(t.dot_id)}`;
    case "memory":
      return `/dots/${e(t.dot_id)}?tab=memory`;
    case "intent":
      return "/intents";
  }
}

/// The groups of the results, in this order, each with its heading.
export const KIND_LABELS: Record<SearchKind, string> = {
  topic: "Topics",
  chat: "You said",
  reply: "Sage said",
  note: "Notes and questions",
  intent: "Goals",
  memory: "Memory",
  file: "Files",
};

export type HitGroup = { kind: SearchKind; label: string; hits: SearchHit[] };

/// `hits` grouped by kind in the order of `KIND_LABELS`, each group keeping their rank.
export function groupHits(hits: SearchHit[]): HitGroup[] {
  return (Object.keys(KIND_LABELS) as SearchKind[])
    .map((kind) => ({ kind, label: KIND_LABELS[kind], hits: hits.filter((h) => h.kind === kind) }))
    .filter((group) => group.hits.length > 0);
}
