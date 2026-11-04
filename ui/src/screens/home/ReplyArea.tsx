import { useMutation, useQueryClient } from "@tanstack/react-query";
import { X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router";

import { commands, toCommandError } from "@/api/commands";
import type { Message, StagedAttachment } from "@/api/types";
import { Button } from "@/components/ui/button";
import { pastedName, room, TOO_MANY } from "@/lib/attachments";
import { orgRemovedText, useDotOrg } from "@/lib/dot-org";
import { useFileDrop } from "@/lib/file-drop";
import {
  defaultTarget,
  isActive,
  lastRun,
  topicTitle,
  type Topic,
} from "@/lib/home-topics";
import { isOpenQuestion } from "@/lib/messages";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";
import { Composer, type ComposerFile } from "@/screens/home/Composer";

/// The core's text for a staged file that is gone or was taken by another send.
const GONE = "An attached file is no longer there. Remove it and attach it again.";

/// A file in the chat box: its chip, and its staged id once the core has it.
type Staged = ComposerFile & { id: string | null };

/// The files in the chat box (attachments spec sections 3 and 6): each is staged by the core the
/// moment it is added. Refusals say why a file was not added; they clear on the next add or a
/// send. Files the box still holds when it goes away are removed from the core.
function useStagedFiles(dotId: string) {
  const [files, setFiles] = useState<Staged[]>([]);
  const [refusals, setRefusals] = useState<string[]>([]);
  const current = useRef<Staged[]>([]);
  const next = useRef(0);
  const mounted = useRef(true);
  const update = (change: (files: Staged[]) => Staged[]) => {
    current.current = change(current.current);
    setFiles(current.current);
  };
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      for (const f of current.current) {
        if (f.id !== null) void commands.attachmentRemove(f.id).catch(() => {});
      }
      current.current = [];
    };
  }, []);
  function add(items: { name: string; stage: () => Promise<StagedAttachment> }[]) {
    const { take, refused } = room(current.current.length, items.length);
    const said: string[] = refused ? [TOO_MANY] : [];
    setRefusals(said);
    for (const item of items.slice(0, take)) {
      const key = `f${next.current++}`;
      update((fs) => [...fs, { key, name: item.name, size: null, id: null }]);
      item.stage().then(
        (staged) => {
          if (!mounted.current) {
            void commands.attachmentRemove(staged.id).catch(() => {});
            return;
          }
          update((fs) =>
            fs.map((f) => (f.key === key ? { key, name: staged.name, size: staged.size, id: staged.id } : f)),
          );
        },
        (error: unknown) => {
          if (!mounted.current) return;
          update((fs) => fs.filter((f) => f.key !== key));
          setRefusals((rs) => [...rs, toCommandError(error).message]);
        },
      );
    }
  }
  return {
    files,
    refusals,
    ids: () => current.current.flatMap((f) => (f.id === null ? [] : [f.id])),
    addPaths: (paths: string[]) =>
      add(
        paths.map((path) => ({
          name: path.split(/[\\/]/).pop() ?? path,
          stage: () => commands.attachmentAddPath(dotId, path),
        })),
      ),
    addFiles: (pasted: File[]) =>
      add(
        pasted.map((file) => {
          const name = pastedName(file);
          return {
            name,
            stage: async () =>
              commands.attachmentAddBytes(dotId, name, new Uint8Array(await file.arrayBuffer())),
          };
        }),
      ),
    remove: (key: string) => {
      const f = current.current.find((x) => x.key === key);
      if (f === undefined || f.id === null) return;
      update((fs) => fs.filter((x) => x.key !== key));
      void commands.attachmentRemove(f.id).catch(() => {});
    },
    /// The files went with a message: the box is empty again.
    sent: () => {
      update(() => []);
      setRefusals([]);
    },
    refuse: (text: string) => setRefusals([text]),
    clearRefusals: () => setRefusals([]),
  };
}

/// Which topic the next message goes to: the default, one the person chose with Reply, or a
/// new topic (chat-first home spec, section 5).
export type ReplyPick = { kind: "default" } | { kind: "topic"; rootId: string } | { kind: "new" };

/// The topic `pick` names among `shown`; undefined for a new topic. By default an older topic
/// shown alone (`olderRoot`), else the current chat topic. A chosen topic that is no longer
/// shown falls back to the default.
export function replyTarget(
  pick: ReplyPick,
  shown: Topic[],
  olderRoot?: string,
): Topic | undefined {
  if (pick.kind === "new") return undefined;
  const chosen = pick.kind === "topic" ? shown.find((t) => t.rootId === pick.rootId) : undefined;
  const older = olderRoot !== undefined ? shown.find((t) => t.rootId === olderRoot) : undefined;
  return chosen ?? older ?? defaultTarget(shown);
}

/// Below the conversation, in its centred column: why the target cannot be continued, the open
/// question, the reply target, and the composer. A message continues the target's last run, or starts a new chat
/// topic without one; an answer to the open question goes through `message_reply`.
export function ReplyArea(props: {
  dotId: string;
  dotName: string;
  /// The dot's org, null for Personal. A dot whose org is not joined can't run, so nothing is
  /// sent: the core would refuse the run at its start anyway.
  orgId: string | null;
  shown: Topic[];
  checkIns: Message[];
  pick: ReplyPick;
  onPick: (pick: ReplyPick) => void;
  /// The older topic shown alone, if it is one.
  olderRoot?: string;
}) {
  const { dotId, dotName, shown, pick, onPick, olderRoot } = props;
  const qc = useQueryClient();
  const navigate = useNavigate();
  const picked = replyTarget(pick, shown, olderRoot);
  // A topic with nothing left to continue (its only message was discarded) gets a new topic.
  const continues = picked !== undefined ? lastRun(picked) : undefined;
  const target = continues !== undefined ? picked : undefined;
  const { removed } = useDotOrg(props.orgId);
  const blocked = removed && props.orgId !== null ? orgRemovedText(props.orgId) : undefined;
  const busy = target?.runs.some(isActive) ?? false;
  const approving = target?.runs.some((run) => run.status === "awaiting_approval") ?? false;
  // A reply resumes the last run's session. A run that ended before its engine had one (a
  // failed start, a cancel while queued) leaves nothing to resume: the core refuses it.
  const closed = continues !== undefined && !busy && continues.session_id === null;
  const runIds = new Set(shown.flatMap((topic) => topic.runs.map((run) => run.id)));
  const open = props.checkIns.filter(
    (m) => m.run_id !== null && runIds.has(m.run_id) && isOpenQuestion(m),
  );
  const question = open.at(-1);
  const questionBusy =
    question !== undefined &&
    shown.some((t) => t.runs.some((r) => r.id === question.run_id) && t.runs.some(isActive));
  // "Reply to question" pressed: the composer sends the answer to the open question.
  const [replying, setReplying] = useState(false);
  const answering = replying && question !== undefined;
  // What a send waits for before it is done: the dot's runs and the threads, loaded again. Until
  // then the screen still ends at the old run, and a second message would continue from it.
  const reload = () =>
    Promise.all([
      qc.invalidateQueries({ queryKey: ["runs"] }),
      qc.invalidateQueries({ queryKey: ["thread"] }),
    ]);
  // An answer or a message that went to another topic than the older one shown is seen in the
  // conversation.
  const follow = (rootId: string) => {
    if (olderRoot !== undefined && rootId !== olderRoot) void navigate(`/home/${dotId}`);
  };
  const staged = useStagedFiles(dotId);
  const send = useMutation({
    mutationFn: (message: string) =>
      commands.chat(dotId, message, continues?.id ?? null, staged.ids()),
    onSuccess: (run) => {
      staged.sent();
      onPick({ kind: "default" });
      follow(run.root_run_id);
      return reload();
    },
    // The core refuses a message when the topic it holds is ahead of this one. So the topics
    // are loaded again after any refusal, and the composer follows what they bring.
    onError: (error) => {
      // A staged file that is gone is said under the chips, where it can be removed.
      if (toCommandError(error).message === GONE) staged.refuse(GONE);
      else notifyError(error);
      return reload();
    },
  });
  // Send later: the text in the box goes to the reply target at `at`. A refusal is shown by
  // the menu that asked (a notice, or the dialog's alert).
  const schedule = useMutation({
    mutationFn: ({ text, at }: { text: string; at: Date }) =>
      commands.scheduleMessage(dotId, text, target?.rootId ?? null, at.toISOString()),
    onSuccess: () => {
      onPick({ kind: "default" });
      invalidate(qc, keys.scheduledMessages(dotId));
    },
  });
  const answer = useMutation({
    mutationFn: (text: string) => commands.messageReply(question!.id, text),
    onSuccess: (run) => {
      setReplying(false);
      invalidate(qc, ["messages"]);
      follow(run.root_run_id);
      return reload();
    },
    onError: (error) => {
      notifyError(error);
      invalidate(qc, ["messages"]);
      return reload();
    },
  });
  // Opening the conversation marks its open questions read; they stay open until answered.
  const unread = open
    .filter((m) => m.read_at === null)
    .map((m) => m.id)
    .join(" ");
  useEffect(() => {
    if (unread === "") return;
    for (const id of unread.split(" ")) {
      void commands.messageMarkRead(id).then(
        () => invalidate(qc, ["messages"]),
        () => {},
      );
    }
  }, [unread, qc]);

  const chip =
    target === undefined
      ? "New topic"
      : olderRoot === undefined && target.kind === "chat"
        ? "Replying to: current topic"
        : `Replying to: ${topicTitle(target)}`;
  const hint = blocked
    ? "account removed"
    : answering
      ? "Your answer to the question"
      : busy
        ? approving
          ? "Waiting for your approval above"
          : `${dotName} is working on this…`
        : closed
          ? "This topic cannot be continued"
          : `Message ${dotName}…`;
  const composerDisabled =
    blocked !== undefined ||
    (answering ? questionBusy || answer.isPending : busy || send.isPending || closed);
  // Files go with messages, not answers; and only into a box that can send.
  const canAttach = !answering;
  const dragging = useFileDrop(canAttach && !composerDisabled, staged.addPaths);
  return (
    <div className="mx-auto flex w-full max-w-[720px] flex-col gap-2 px-4 pb-4">
      {blocked && (
        <p role="status" className="px-1 text-destructive">
          account removed: {blocked}
        </p>
      )}
      {closed && (
        <p className="flex items-center gap-2 px-1 text-muted-foreground">
          <span className="flex-1">
            This topic cannot be continued: its last run ended before the dot had a session to
            resume.
          </span>
          <Button size="sm" variant="outline" onClick={() => onPick({ kind: "new" })}>
            New topic
          </Button>
        </p>
      )}
      {question && (
        <div
          role="region"
          aria-label="Open question"
          className="flex flex-wrap items-center gap-2 rounded-xl border bg-card px-3 py-2"
        >
          <p className="min-w-0 flex-1 whitespace-pre-wrap">
            {dotName} asked: {question.text}
          </p>
          {question.choices?.map((choice) => (
            <Button
              key={choice}
              size="sm"
              variant="outline"
              disabled={questionBusy || answer.isPending || blocked !== undefined}
              onClick={onSingleClick(() => answer.mutate(choice))}
            >
              {choice}
            </Button>
          ))}
          <Button
            size="sm"
            variant={answering ? "default" : "outline"}
            aria-pressed={answering}
            disabled={questionBusy || blocked !== undefined}
            onClick={() => setReplying(!replying)}
          >
            Reply to question
          </Button>
        </div>
      )}
      <div
        role="group"
        aria-label="Reply target"
        aria-live="polite"
        className="flex items-center gap-1 px-1 text-xs text-muted-foreground"
      >
        <span>{chip}</span>
        {pick.kind !== "default" && (
          <Button
            size="icon-xs"
            variant="ghost"
            aria-label="Clear the reply target"
            title="Clear the reply target"
            onClick={() => onPick({ kind: "default" })}
          >
            <X aria-hidden />
          </Button>
        )}
      </div>
      <Composer
        disabled={composerDisabled}
        hint={hint}
        files={staged.files}
        onAddPaths={staged.addPaths}
        onAddFiles={staged.addFiles}
        onRemove={staged.remove}
        refusals={staged.refusals}
        canAttach={canAttach}
        dragging={dragging}
        onSend={(message) =>
          answering ? answer.mutateAsync(message) : send.mutateAsync(message)
        }
        onSchedule={
          answering ? undefined : (text, at) => schedule.mutateAsync({ text, at })
        }
      />
    </div>
  );
}
