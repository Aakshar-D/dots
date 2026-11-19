import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Brain, FolderOpen, Pin } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import { LIMITS, type Memory, type MemoryKind } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { EmptyState } from "@/components/EmptyState";
import { ErrorNote } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { Textarea } from "@/components/ui/textarea";
import { localDateTime } from "@/lib/format";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";

/// The four kinds of memory (memory spec section 2), in the order the tab groups them: the
/// group's heading, and the words of the kind's choice in the box.
const KINDS: { kind: MemoryKind; heading: string; choice: string }[] = [
  { kind: "core", heading: "Core", choice: "Core: in every prompt" },
  { kind: "semantic", heading: "Facts and decisions", choice: "Fact or decision" },
  { kind: "procedural", heading: "How-to", choice: "How-to" },
  { kind: "episodic", heading: "What runs did", choice: "What a run did" },
];

/// The characters of a memory's text as the core counts them: trimmed, by code point.
function length(text: string): number {
  return [...text.trim()].length;
}

/// The most characters a memory of `kind` may have.
function maxOf(kind: MemoryKind): number {
  return kind === "core" ? LIMITS.max_core_memory_text : LIMITS.max_memory_text;
}

/// Whether `memory` holds every word of `query`, in any case.
function matches(memory: Memory, query: string): boolean {
  const text = memory.text.toLowerCase();
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((word) => text.includes(word));
}

/// The box a memory is written in (Add memory, Edit): its text, its kind, a counter of the
/// characters the kind allows, Save and Cancel; Escape cancels. The counter and the core's
/// refusal, in its words, describe the text box.
function MemoryBox(props: {
  label: string;
  initial: string;
  kind: MemoryKind;
  pending: boolean;
  error: unknown;
  onSave: (text: string, kind: MemoryKind) => void;
  onCancel: () => void;
}) {
  const [text, setText] = useState(props.initial);
  const [kind, setKind] = useState(props.kind);
  const used = length(text);
  const max = maxOf(kind);
  const over = used > max;
  const refused = props.error !== null;
  const counterId = useId();
  const errorId = useId();
  return (
    <div
      role="group"
      aria-label={props.label}
      className="flex flex-col gap-2 rounded-lg border p-3"
      onKeyDown={(e) => {
        if (e.key !== "Escape") return;
        e.preventDefault();
        props.onCancel();
      }}
    >
      <Textarea
        aria-label={props.label}
        value={text}
        autoFocus
        aria-invalid={over || refused || undefined}
        aria-describedby={refused ? `${counterId} ${errorId}` : counterId}
        onChange={(e) => setText(e.target.value)}
      />
      <div className="flex items-center gap-2">
        <NativeSelect
          size="sm"
          aria-label="Kind"
          value={kind}
          onChange={(e) => setKind(e.target.value as MemoryKind)}
        >
          {KINDS.map((k) => (
            <NativeSelectOption key={k.kind} value={k.kind}>
              {k.choice}
            </NativeSelectOption>
          ))}
        </NativeSelect>
        <span
          id={counterId}
          aria-live="polite"
          className={over ? "text-xs text-destructive" : "text-xs text-muted-foreground"}
        >
          {used} / {max}
        </span>
        <span className="flex-1" />
        <Button size="sm" variant="outline" onClick={props.onCancel}>
          Cancel
        </Button>
        <Button
          size="sm"
          disabled={props.pending || used === 0 || over}
          onClick={onSingleClick(() => props.onSave(text, kind))}
        >
          Save
        </Button>
      </div>
      {props.error !== null && (
        <p id={errorId} role="alert" className="text-sm text-destructive">
          {errorMessage(props.error)}
        </p>
      )}
    </div>
  );
}

/// Who wrote a memory: a sol in a run (a link to it), Sage after a run, or the person.
function Byline({ memory }: { memory: Memory }) {
  if (memory.source === "person" || memory.run_id === null) {
    return <span className="text-xs text-muted-foreground">by you</span>;
  }
  const who =
    memory.source === "capture"
      ? "saved by Sage after run"
      : memory.scope === "project"
        ? "by a sol · from run"
        : "by the sol · from run";
  return (
    <span className="text-xs text-muted-foreground">
      {who}{" "}
      <Link to={`/runs/${memory.run_id}`} className="text-foreground hover:underline">
        {localDateTime(memory.created_at)}
      </Link>
    </span>
  );
}

/// Where a list's memories are kept: the commands that change them, and its own words.
type Source = {
  /// The list's name: "Memories", "Project memories".
  name: string;
  searchLabel: string;
  addLabel: string;
  empty: string;
  max: number;
  add: (text: string, kind: MemoryKind) => Promise<Memory>;
  /// A part left null stays as it is.
  update: (
    id: string,
    text: string | null,
    pinned: boolean | null,
    kind: MemoryKind | null,
  ) => Promise<Memory>;
  remove: (id: string) => Promise<null>;
  refresh: () => void;
};

/// One list of memories (memory spec section 11): by kind, each kind pinned first, then
/// newest, with a search, the count, and Add memory; each memory can be pinned, edited and
/// deleted. A memory's text is shown as plain text. After a save, a cancel or a delete, focus
/// goes back where the person was: Add memory, the memory's Edit, or after a delete the next
/// memory's Edit.
function MemoryList({ source, memories }: { source: Source; memories: Memory[] }) {
  const [query, setQuery] = useState("");
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<string | null>(null);
  // Where focus goes once the list shows it: "add" (Add memory) or a memory's id (its Edit).
  const [focusTo, setFocusTo] = useState<string | null>(null);
  // The memory being deleted: focus moves on only once the list no longer shows it. The
  // dialog that asked gives focus back to its Delete button when it closes, and focus set
  // before that row goes would be lost with it.
  const leaving = useRef<string | null>(null);
  const addButton = useRef<HTMLButtonElement>(null);
  const searchBox = useRef<HTMLInputElement>(null);
  const editButtons = useRef(new Map<string, HTMLButtonElement>());
  const add = useMutation({
    mutationFn: (v: { text: string; kind: MemoryKind }) => source.add(v.text, v.kind),
    onSuccess: () => {
      setAdding(false);
      setFocusTo("add");
    },
    onSettled: source.refresh,
  });
  const edit = useMutation({
    mutationFn: (v: { id: string; text: string; kind: MemoryKind | null }) =>
      source.update(v.id, v.text, null, v.kind),
    onSuccess: (_, v) => {
      setEditing(null);
      setFocusTo(v.id);
    },
    onSettled: source.refresh,
  });
  const pin = useMutation({
    mutationFn: (v: { id: string; pinned: boolean }) => source.update(v.id, null, v.pinned, null),
    onError: notifyError,
    onSettled: source.refresh,
  });
  const remove = useMutation({
    mutationFn: (v: { id: string; then: string }) => source.remove(v.id),
    onSuccess: (_, v) => {
      leaving.current = v.id;
      setFocusTo(v.then);
    },
    onError: notifyError,
    onSettled: source.refresh,
  });

  useEffect(() => {
    if (focusTo === null) return;
    if (leaving.current !== null && memories.some((m) => m.id === leaving.current)) return;
    leaving.current = null;
    const add = addButton.current;
    const target =
      focusTo === "add"
        ? add && !add.disabled
          ? add
          : searchBox.current
        : editButtons.current.get(focusTo);
    if (!target) return;
    target.focus();
    setFocusTo(null);
  });

  // By kind; within a kind the order the core gave: pinned first, then newest.
  const shown = KINDS.flatMap(({ kind }) =>
    memories.filter((m) => m.kind === kind && matches(m, query)),
  );
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2">
        <Input
          ref={searchBox}
          type="search"
          aria-label={source.searchLabel}
          placeholder="Search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          className="max-w-xs"
        />
        <span className="flex-1 text-sm text-muted-foreground">
          {memories.length} of {source.max}
        </span>
        <Button
          ref={addButton}
          size="sm"
          disabled={adding || memories.length >= source.max}
          onClick={() => {
            add.reset();
            setAdding(true);
          }}
        >
          {source.addLabel}
        </Button>
      </div>
      {adding && (
        <MemoryBox
          label="New memory"
          initial=""
          kind="semantic"
          pending={add.isPending}
          error={add.error}
          onSave={(text, kind) => add.mutate({ text, kind })}
          onCancel={() => {
            setAdding(false);
            setFocusTo("add");
          }}
        />
      )}
      {memories.length === 0 ? (
        <EmptyState icon={Brain}>{source.empty}</EmptyState>
      ) : shown.length === 0 ? (
        <p className="text-muted-foreground">No memory has those words.</p>
      ) : (
        <ul aria-label={source.name} className="flex flex-col gap-2">
          {/* One flat list of keyed rows, so a row stays the same element (and keeps focus)
              when a heading above it comes or goes. */}
          {shown.flatMap((memory, i) => {
            const heading =
              shown[i - 1]?.kind === memory.kind
                ? null
                : KINDS.find((k) => k.kind === memory.kind)!.heading;
            return [
              heading !== null && (
                <li
                  key={`kind-${memory.kind}`}
                  role="presentation"
                  className="pt-1 text-xs font-medium text-muted-foreground"
                >
                  <span role="heading" aria-level={4}>
                    {heading}
                  </span>
                </li>
              ),
              editing === memory.id ? (
                <li key={memory.id}>
                  <MemoryBox
                    label="Edit memory"
                    initial={memory.text}
                    kind={memory.kind}
                    pending={edit.isPending}
                    error={edit.error}
                    onSave={(text, kind) =>
                      edit.mutate({
                        id: memory.id,
                        text,
                        kind: kind === memory.kind ? null : kind,
                      })
                    }
                    onCancel={() => {
                      setEditing(null);
                      setFocusTo(memory.id);
                    }}
                  />
                </li>
              ) : (
                <li
                  key={memory.id}
                  aria-label={memory.text}
                  className="flex items-start gap-3 rounded-lg border p-3"
                >
                  <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <p className="whitespace-pre-wrap break-words">{memory.text}</p>
                    {memory.files.length > 0 && (
                      <p className="truncate font-mono text-xs text-muted-foreground">
                        About {memory.files.join(", ")}
                      </p>
                    )}
                    <Byline memory={memory} />
                  </div>
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    aria-label="Pinned"
                    title={memory.pinned ? "Unpin" : "Pin"}
                    aria-pressed={memory.pinned}
                    disabled={pin.isPending}
                    onClick={() => pin.mutate({ id: memory.id, pinned: !memory.pinned })}
                  >
                    <Pin
                      aria-hidden
                      className={
                        memory.pinned ? "fill-current text-primary" : "text-muted-foreground"
                      }
                    />
                  </Button>
                  <Button
                    ref={(el) => {
                      if (el) editButtons.current.set(memory.id, el);
                      else editButtons.current.delete(memory.id);
                    }}
                    size="sm"
                    variant="outline"
                    onClick={() => {
                      edit.reset();
                      setEditing(memory.id);
                    }}
                  >
                    Edit
                  </Button>
                  <ConfirmButton
                    title="Delete this memory?"
                    description={memory.text}
                    confirmLabel="Delete"
                    disabled={remove.isPending}
                    onConfirm={() =>
                      remove.mutate({
                        id: memory.id,
                        then: (shown[i + 1] ?? shown[i - 1])?.id ?? "add",
                      })
                    }
                  >
                    Delete
                  </ConfirmButton>
                </li>
              ),
            ];
          })}
        </ul>
      )}
    </div>
  );
}

/// What the sol itself remembers. The core announces every change (`memories_updated`), so the
/// list refreshes itself.
function SolMemory({ dotId }: { dotId: string }) {
  const qc = useQueryClient();
  const list = useQuery({
    queryKey: keys.memories(dotId),
    queryFn: () => commands.memoriesList(dotId),
  });
  if (list.isPending) return null;
  if (list.isError) return <ErrorNote error={list.error} />;
  const source: Source = {
    name: "Memories",
    searchLabel: "Search memories",
    addLabel: "Add memory",
    empty: "Nothing remembered yet. The sol saves what it learns here; you can add notes too.",
    max: LIMITS.max_memories,
    // The kind is sent only when it is not the usual one, and only when it changed.
    add: (text, kind) => commands.memoryAdd(dotId, text, kind === "semantic" ? undefined : kind),
    update: (id, text, pinned, kind) => commands.memoryUpdate(id, text, pinned, kind ?? undefined),
    remove: (id) => commands.memoryDelete(id),
    refresh: () => invalidate(qc, keys.memories(dotId)),
  };
  return <MemoryList source={source} memories={list.data} />;
}

/// What the project the sol works in remembers (memory spec section 4): the folder its
/// memory files are in, with a way to open it, and the facts read from those files. Every sol
/// that works in the project shares them. Nothing is shown for a sol whose folder is missing.
function ProjectMemory({ dotId }: { dotId: string }) {
  const qc = useQueryClient();
  const project = useQuery({
    queryKey: keys.projectMemories(dotId),
    queryFn: () => commands.projectMemories(dotId),
  });
  const open = useMutation({
    mutationFn: () => commands.openMemoryFolder(dotId),
    onError: notifyError,
  });
  if (project.isPending) return null;
  if (project.isError) return <ErrorNote error={project.error} />;
  if (project.data === null) {
    return (
      <p className="text-sm text-muted-foreground">
        This sol's folder is missing, so it has no project memory.
      </p>
    );
  }
  const { dir, memories, skipped } = project.data;
  const source: Source = {
    name: "Project memories",
    searchLabel: "Search project memories",
    addLabel: "Add project memory",
    empty: "Nothing remembered for this project yet. Sage and the sols that work here save facts as files in this folder.",
    max: LIMITS.max_project_memories,
    add: (text, kind) => commands.projectMemoryAdd(dotId, text, kind, []),
    update: (id, text, pinned, kind) => commands.projectMemoryUpdate(dotId, id, text, pinned, kind),
    remove: (id) => commands.projectMemoryDelete(dotId, id),
    refresh: () => invalidate(qc, keys.projectMemories(dotId)),
  };
  return (
    <>
      <div className="flex items-center gap-2">
        <p className="min-w-0 flex-1 truncate font-mono text-xs" title={dir}>
          {dir}
        </p>
        <Button
          size="sm"
          variant="outline"
          disabled={open.isPending}
          onClick={() => open.mutate()}
        >
          <FolderOpen aria-hidden />
          Open folder
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        Every sol that works in this project reads and writes these. Each fact is a file you can
        edit or delete. Commit .sage/memory to share them, or add .sage/ to .gitignore to keep
        them on this computer.
      </p>
      {skipped > 0 && (
        <p className="text-xs text-muted-foreground">
          {skipped === 1
            ? "1 file in the folder is not a memory Sage can read."
            : `${skipped} files in the folder are not memories Sage can read.`}
        </p>
      )}
      <MemoryList source={source} memories={memories} />
    </>
  );
}

/// The dot's Memory tab (dot memory spec section 5, memory spec section 11): what the sol
/// remembers, and what the project it works in remembers.
export function MemoryTab({ dotId }: { dotId: string }) {
  return (
    <div className="flex flex-col gap-8">
      <section aria-label="This sol's memory" className="flex flex-col gap-3">
        <h3 className="text-sm font-medium">This sol</h3>
        <SolMemory dotId={dotId} />
      </section>
      <section aria-label="The project's memory" className="flex flex-col gap-3">
        <h3 className="text-sm font-medium">The project</h3>
        <ProjectMemory dotId={dotId} />
      </section>
    </div>
  );
}
