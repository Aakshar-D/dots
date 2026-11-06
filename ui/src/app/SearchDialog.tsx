import { useQuery } from "@tanstack/react-query";
import { cn } from "cn";
import { Search } from "lucide-react";
import { useEffect, useId, useState } from "react";
import { useNavigate } from "react-router";

import { commands } from "@/api/commands";
import type { SearchHit } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { relativeTime } from "@/lib/format";
import { keys } from "@/lib/query";
import { useCurrentScope } from "@/lib/scope";
import { groupHits, hitPath, snippetParts } from "@/lib/search";
import { useDebounced } from "@/lib/use-debounced";

/// How long the box waits after the last key before it searches.
export const SEARCH_DEBOUNCE_MS = 200;

/// Whether `text` has a letter or a digit: marks alone search for nothing.
function hasWord(text: string): boolean {
  return /[\p{L}\p{N}]/u.test(text);
}

/// Whether `e` is the search shortcut: Ctrl+K, or Cmd+K on a Mac.
export function isSearchShortcut(e: KeyboardEvent): boolean {
  return (e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "k";
}

/// A snippet with its matched words marked; the words are text, never markup.
function Snippet({ text }: { text: string }) {
  return (
    <>
      {snippetParts(text).map((part, i) =>
        part.hit ? (
          <mark key={i} className="rounded-sm bg-primary/15 px-0.5 text-foreground">
            {part.text}
          </mark>
        ) : (
          <span key={i}>{part.text}</span>
        ),
      )}
    </>
  );
}

/// The search box (search spec section 5): what the person types searches the chats, results,
/// memory, goals, topic titles and chat files of the workspace in view, after a short pause.
/// The hits come grouped by kind; the arrow keys move through them, Enter or a click opens one.
export function SearchDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [text, setText] = useState("");
  const [active, setActive] = useState(0);
  const navigate = useNavigate();
  const scope = useCurrentScope();
  const settled = useDebounced(text.trim(), SEARCH_DEBOUNCE_MS);
  const listId = useId();
  const optionId = (i: number) => `${listId}-${i}`;
  const results = useQuery({
    queryKey: keys.search(scope?.account ?? "", scope?.workspace ?? "", settled),
    queryFn: () =>
      commands.search(scope!.account, scope!.workspace, {
        text: settled,
        kinds: [],
        dot_id: null,
        limit: null,
      }),
    enabled: open && scope !== undefined && hasWord(settled),
    staleTime: 5_000,
  });
  // Each group with the place of its first hit among all of them.
  let start = 0;
  const groups = groupHits(results.data ?? []).map((group) => {
    const first = start;
    start += group.hits.length;
    return { ...group, first };
  });
  const flat: SearchHit[] = groups.flatMap((g) => g.hits);

  // A new set of hits starts at the first.
  useEffect(() => setActive(0), [results.data]);
  // The box opens empty each time.
  useEffect(() => {
    if (!open) setText("");
  }, [open]);

  const openHit = (hit: SearchHit) => {
    onOpenChange(false);
    void navigate(hitPath(hit));
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (flat.length === 0) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((i) => (i + 1) % flat.length);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((i) => (i - 1 + flat.length) % flat.length);
    } else if (e.key === "Home") {
      e.preventDefault();
      setActive(0);
    } else if (e.key === "End") {
      e.preventDefault();
      setActive(flat.length - 1);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const hit = flat[active];
      if (hit) openHit(hit);
    }
  };

  // Keep the active hit in view.
  useEffect(() => {
    document.getElementById(optionId(active))?.scrollIntoView?.({ block: "nearest" });
  });

  const waiting = text.trim() !== settled || results.isFetching;
  const searched = results.isSuccess && hasWord(settled);
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="top-24 translate-y-0 gap-3 sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>Search</DialogTitle>
          <DialogDescription>
            Chats, results, memory, goals, topics and files in this workspace.
          </DialogDescription>
        </DialogHeader>
        <div className="relative">
          <Search
            aria-hidden
            className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
          />
          <Input
            role="combobox"
            aria-label="Search everything"
            aria-expanded={flat.length > 0}
            aria-controls={listId}
            aria-autocomplete="list"
            aria-activedescendant={flat.length > 0 ? optionId(active) : undefined}
            autoComplete="off"
            spellCheck={false}
            placeholder="Search…"
            className="pl-8"
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={onKeyDown}
          />
        </div>
        <div className="max-h-[60vh] min-h-12 overflow-y-auto">
          {results.isError ? (
            <ErrorNote error={results.error} />
          ) : !hasWord(settled) && !hasWord(text) ? (
            <p className="text-muted-foreground">
              Type a word to find it. Ctrl+K opens this box from anywhere.
            </p>
          ) : searched && flat.length === 0 && !waiting ? (
            <p role="status" className="text-muted-foreground">
              Nothing matches “{settled}”.
            </p>
          ) : null}
          {waiting && flat.length === 0 && !results.isError && hasWord(text) && (
            <p role="status" className="text-muted-foreground">
              Searching…
            </p>
          )}
          <div id={listId} role="listbox" aria-label="Results" hidden={flat.length === 0}>
            {groups.map((group) => {
              const headingId = `${listId}-${group.kind}`;
              return (
                <div key={group.kind} role="group" aria-labelledby={headingId} className="mb-2">
                  <div
                    id={headingId}
                    role="presentation"
                    className="px-2 py-1 text-xs font-medium text-muted-foreground"
                  >
                    {group.label}
                  </div>
                  {group.hits.map((hit, n) => {
                    const i = group.first + n;
                    return (
                      <div
                        key={i}
                        id={optionId(i)}
                        role="option"
                        aria-selected={i === active}
                        tabIndex={-1}
                        onMouseMove={() => setActive(i)}
                        onClick={() => openHit(hit)}
                        className={cn(
                          "cursor-pointer rounded-md px-2 py-1.5",
                          i === active && "bg-accent text-accent-foreground",
                        )}
                      >
                        <div className="line-clamp-2">
                          <Snippet text={hit.snippet} />
                        </div>
                        <div className="text-xs text-muted-foreground">
                          {[hit.dot_name, relativeTime(hit.at)].filter(Boolean).join(" · ")}
                        </div>
                      </div>
                    );
                  })}
                </div>
              );
            })}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
