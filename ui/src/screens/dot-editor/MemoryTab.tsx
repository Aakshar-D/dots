import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Brain, Pin } from "lucide-react";
import { useState } from "react";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import { LIMITS, type Memory } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { EmptyState } from "@/components/EmptyState";
import { ErrorNote } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { localDateTime } from "@/lib/format";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";

/// The characters of a memory's text as the core counts them: trimmed, by code point.
function length(text: string): number {
  return [...text.trim()].length;
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

/// The box a memory is written in (Add memory, Edit): its text, a counter of the 500
/// characters, Save and Cancel. The core's refusal shows under it, in its words.
function MemoryBox(props: {
  label: string;
  initial: string;
  pending: boolean;
  error: unknown;
  onSave: (text: string) => void;
  onCancel: () => void;
}) {
  const [text, setText] = useState(props.initial);
  const used = length(text);
  const max = LIMITS.max_memory_text;
  const over = used > max;
  return (
    <div
      role="group"
      aria-label={props.label}
      className="flex flex-col gap-2 rounded-lg border p-3"
    >
      <Textarea
        aria-label={props.label}
        value={text}
        autoFocus
        aria-invalid={over || undefined}
        onChange={(e) => setText(e.target.value)}
      />
      <div className="flex items-center gap-2">
        <span
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
          onClick={onSingleClick(() => props.onSave(text))}
        >
          Save
        </Button>
      </div>
      {props.error !== null && (
        <p role="alert" className="text-sm text-destructive">
          {errorMessage(props.error)}
        </p>
      )}
    </div>
  );
}

/// Who wrote a memory: the dot in a run (a link to it) or the person.
function Byline({ memory }: { memory: Memory }) {
  if (memory.source === "person" || memory.run_id === null) {
    return <span className="text-xs text-muted-foreground">by you</span>;
  }
  return (
    <span className="text-xs text-muted-foreground">
      by the dot · from run{" "}
      <Link to={`/runs/${memory.run_id}`} className="text-foreground hover:underline">
        {localDateTime(memory.created_at)}
      </Link>
    </span>
  );
}

/// The dot's Memory tab (dot memory spec section 5): what the dot remembers, pinned first, then
/// newest, with a search, the count, and Add memory; each memory can be pinned, edited and
/// deleted. The core announces every change (`memories_updated`), so the list refreshes itself.
export function MemoryTab({ dotId }: { dotId: string }) {
  const qc = useQueryClient();
  const list = useQuery({
    queryKey: keys.memories(dotId),
    queryFn: () => commands.memoriesList(dotId),
  });
  const [query, setQuery] = useState("");
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<string | null>(null);
  const refresh = () => invalidate(qc, keys.memories(dotId));
  const add = useMutation({
    mutationFn: (text: string) => commands.memoryAdd(dotId, text),
    onSuccess: () => setAdding(false),
    onSettled: refresh,
  });
  const edit = useMutation({
    mutationFn: (v: { id: string; text: string }) => commands.memoryUpdate(v.id, v.text, null),
    onSuccess: () => setEditing(null),
    onSettled: refresh,
  });
  const pin = useMutation({
    mutationFn: (v: { id: string; pinned: boolean }) =>
      commands.memoryUpdate(v.id, null, v.pinned),
    onError: notifyError,
    onSettled: refresh,
  });
  const remove = useMutation({
    mutationFn: (id: string) => commands.memoryDelete(id),
    onError: notifyError,
    onSettled: refresh,
  });

  if (list.isPending) return null;
  if (list.isError) return <ErrorNote error={list.error} />;
  const memories = list.data;
  const shown = memories.filter((m) => matches(m, query));
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2">
        <Input
          type="search"
          aria-label="Search memories"
          placeholder="Search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          className="max-w-xs"
        />
        <span className="flex-1 text-sm text-muted-foreground">
          {memories.length} of {LIMITS.max_memories}
        </span>
        <Button
          size="sm"
          disabled={adding}
          onClick={() => {
            add.reset();
            setAdding(true);
          }}
        >
          Add memory
        </Button>
      </div>
      {adding && (
        <MemoryBox
          label="New memory"
          initial=""
          pending={add.isPending}
          error={add.error}
          onSave={(text) => add.mutate(text)}
          onCancel={() => setAdding(false)}
        />
      )}
      {memories.length === 0 ? (
        <EmptyState icon={Brain}>
          Nothing remembered yet. The dot saves what it learns here; you can add notes too.
        </EmptyState>
      ) : shown.length === 0 ? (
        <p className="text-muted-foreground">No memory has those words.</p>
      ) : (
        <ul aria-label="Memories" className="flex flex-col gap-2">
          {shown.map((memory) =>
            editing === memory.id ? (
              <li key={memory.id}>
                <MemoryBox
                  label="Memory"
                  initial={memory.text}
                  pending={edit.isPending}
                  error={edit.error}
                  onSave={(text) => edit.mutate({ id: memory.id, text })}
                  onCancel={() => setEditing(null)}
                />
              </li>
            ) : (
              <li
                key={memory.id}
                aria-label={memory.text}
                className="flex items-start gap-3 rounded-lg border p-3"
              >
                <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <p className="whitespace-pre-wrap">{memory.text}</p>
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
                    className={memory.pinned ? "fill-current text-primary" : "text-muted-foreground"}
                  />
                </Button>
                <Button
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
                  onConfirm={() => remove.mutate(memory.id)}
                >
                  Delete
                </ConfirmButton>
              </li>
            ),
          )}
        </ul>
      )}
    </div>
  );
}
