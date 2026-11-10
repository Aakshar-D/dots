import { useQueryClient } from "@tanstack/react-query";
import { MoreHorizontal } from "lucide-react";
import { DropdownMenu } from "radix-ui";
import { useState, type ReactNode } from "react";
import { useNavigate } from "react-router";

import { commands } from "@/api/commands";
import type { Run } from "@/api/types";
import { useContextChoice } from "@/components/ContextMeter";
import { ErrorNote } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { localDateTime } from "@/lib/format";
import { topicTitle, type Topic } from "@/lib/home-topics";
import { invalidate, keys } from "@/lib/query";

const ITEM =
  "flex cursor-default items-center gap-2 rounded-sm px-2 py-1.5 outline-none select-none data-[disabled]:text-muted-foreground data-[highlighted]:bg-accent";

/// What a picker calls a topic: its name and when it began.
const label = (topic: Topic) => `${topicTitle(topic)} · ${localDateTime(topic.root.queued_at)}`;

/// The value the topic pickers give "New topic".
const NEW = "";

/// Refreshes what a change to the dot's topics touches, without waiting for its event.
function useRefresh(dotId: string) {
  const qc = useQueryClient();
  return () => {
    for (const queryKey of [keys.topics(dotId), keys.runs(dotId), ["thread"]]) {
      invalidate(qc, queryKey);
    }
  };
}

/// A dialog with one field and a button, which shows the core's refusal in its words.
function ActionDialog(props: {
  title: string;
  description?: string;
  action: string;
  disabled?: boolean;
  onClose: () => void;
  onSubmit: () => Promise<void>;
  children: ReactNode;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  async function submit() {
    setBusy(true);
    setError(null);
    try {
      await props.onSubmit();
      props.onClose();
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog open onOpenChange={(open) => !open && props.onClose()}>
      <DialogContent>
        <form
          className="flex flex-col gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            void submit();
          }}
        >
          <DialogHeader>
            <DialogTitle>{props.title}</DialogTitle>
            {props.description && <DialogDescription>{props.description}</DialogDescription>}
          </DialogHeader>
          {props.children}
          {error !== null && <ErrorNote error={error} />}
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={props.onClose}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy || props.disabled}>
              {props.action}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/// The menu of one topic (topic tools spec section 6): Rename…, Merge into… one of `others`
/// (the dot's other topics that are not archived), Archive or Unarchive; for a Claude dot
/// (`claude`), Compact now and Start fresh, which its next message does first (context rules
/// spec section 6).
export function TopicMenu({
  dotId,
  topic,
  others,
  claude = false,
}: {
  dotId: string;
  topic: Topic;
  others: Topic[];
  claude?: boolean;
}) {
  const choose = useContextChoice(topic.rootId);
  const navigate = useNavigate();
  const refresh = useRefresh(dotId);
  const [open, setOpen] = useState<"rename" | "merge" | null>(null);
  const [name, setName] = useState("");
  const [into, setInto] = useState("");
  const [error, setError] = useState<unknown>(null);
  async function archive(archived: boolean) {
    setError(null);
    try {
      await commands.setTopicArchived(topic.rootId, archived);
      refresh();
    } catch (e) {
      setError(e);
    }
  }
  return (
    <>
      <DropdownMenu.Root modal={false}>
        <DropdownMenu.Trigger asChild>
          <Button size="sm" variant="ghost" title="Topic" aria-label="Topic">
            <MoreHorizontal aria-hidden />
          </Button>
        </DropdownMenu.Trigger>
        <DropdownMenu.Portal>
          <DropdownMenu.Content
            align="end"
            className="z-50 w-48 rounded-md border bg-popover p-1 text-sm text-popover-foreground shadow-md"
          >
            <DropdownMenu.Item
              className={ITEM}
              onSelect={() => {
                setName(topic.title ?? "");
                setOpen("rename");
              }}
            >
              Rename…
            </DropdownMenu.Item>
            <DropdownMenu.Item
              className={ITEM}
              disabled={others.length === 0}
              onSelect={() => {
                setInto(others.at(-1)?.rootId ?? "");
                setOpen("merge");
              }}
            >
              Merge into…
            </DropdownMenu.Item>
            <DropdownMenu.Item className={ITEM} onSelect={() => void archive(!topic.archived)}>
              {topic.archived ? "Unarchive" : "Archive"}
            </DropdownMenu.Item>
            {claude && (
              <>
                <DropdownMenu.Separator className="my-1 h-px bg-border" />
                <DropdownMenu.Item className={ITEM} onSelect={() => choose.mutate("compact")}>
                  Compact now
                </DropdownMenu.Item>
                <DropdownMenu.Item className={ITEM} onSelect={() => choose.mutate("fresh")}>
                  Start fresh
                </DropdownMenu.Item>
              </>
            )}
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu.Root>
      {error !== null && <ErrorNote error={error} />}
      {open === "rename" && (
        <ActionDialog
          title="Rename topic"
          description="Leave it empty to name it after its first message again."
          action="Save"
          onClose={() => setOpen(null)}
          onSubmit={async () => {
            await commands.renameTopic(topic.rootId, name.trim() === "" ? null : name);
            refresh();
          }}
        >
          <Input
            aria-label="Name"
            autoFocus
            maxLength={100}
            value={name}
            placeholder={topicTitle({ ...topic, title: null })}
            onChange={(e) => setName(e.target.value)}
          />
        </ActionDialog>
      )}
      {open === "merge" && (
        <ActionDialog
          title="Merge into another topic"
          description={`The messages of “${topicTitle(topic)}” join the topic you pick, by time.`}
          action="Merge"
          disabled={into === ""}
          onClose={() => setOpen(null)}
          onSubmit={async () => {
            await commands.mergeTopics(topic.rootId, into);
            refresh();
            // The topic shown alone is gone: show the one it joined.
            void navigate(`/home/${dotId}/${into}`);
          }}
        >
          <NativeSelect
            aria-label="Topic to merge into"
            className="w-full"
            value={into}
            onChange={(e) => setInto(e.target.value)}
          >
            {others.map((other) => (
              <NativeSelectOption key={other.rootId} value={other.rootId}>
                {label(other)}
              </NativeSelectOption>
            ))}
          </NativeSelect>
        </ActionDialog>
      )}
    </>
  );
}

/// "Move to topic…" beside a message the person sent (topic tools spec section 6): the message
/// and its replies go to another of the dot's topics (`topics`, not archived), or to a new one.
export function MoveToTopic({ run, topics }: { run: Run; topics: Topic[] }) {
  const refresh = useRefresh(run.dot_id);
  const [open, setOpen] = useState(false);
  const [to, setTo] = useState(NEW);
  const others = topics.filter((topic) => topic.rootId !== run.root_run_id);
  return (
    <>
      <Button
        size="sm"
        variant="ghost"
        className="h-6 self-end px-1.5 text-xs text-muted-foreground"
        onClick={() => {
          setTo(NEW);
          setOpen(true);
        }}
      >
        Move to topic…
      </Button>
      {open && (
        <ActionDialog
          title="Move to topic"
          description="This message and the replies after it go to the topic you pick."
          action="Move"
          onClose={() => setOpen(false)}
          onSubmit={async () => {
            await commands.moveRun(run.id, to === NEW ? null : to);
            refresh();
          }}
        >
          <NativeSelect
            aria-label="Topic to move to"
            className="w-full"
            value={to}
            onChange={(e) => setTo(e.target.value)}
          >
            <NativeSelectOption value={NEW}>New topic</NativeSelectOption>
            {others.map((other) => (
              <NativeSelectOption key={other.rootId} value={other.rootId}>
                {label(other)}
              </NativeSelectOption>
            ))}
          </NativeSelect>
        </ActionDialog>
      )}
    </>
  );
}
