import { open } from "@tauri-apps/plugin-dialog";
import { cn } from "cn";
import { ArrowUp, Paperclip } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { AttachmentChip } from "@/components/AttachmentChip";
import { Button } from "@/components/ui/button";
import { useComposerReport } from "@/lib/use-activity";
import { SendLaterMenu } from "@/screens/home/SendLaterMenu";

/// Why Send later did nothing: the box was locked, as Send is, or emptied meanwhile.
export const SCHEDULE_LOCKED =
  "You can't schedule a message right now. Try again once you can send one.";
export const SCHEDULE_EMPTY = "Type a message to schedule.";

/// Where a message is typed: one rounded box holding the text and, at its bottom right, the send
/// button (conversational chat spec section 5). The text grows with what is typed, up to a few
/// lines, then scrolls. Enter sends; Shift+Enter starts a new line. The text is cleared once the
/// send has succeeded; a refused message stays for another try. With `onSchedule`, an arrow
/// beside Send schedules the text for later instead (send later spec section 6), usable
/// exactly when Send is; the text is cleared once it is scheduled.
///
/// Files (attachments spec section 6): the paperclip picks files, a paste of files adds them,
/// and each shows as a chip under the text. A message may be files only. Send waits for every
/// file still being added. The caller owns the files (`files`, `onAddPaths`, `onAddFiles`,
/// `onRemove`), says why one was refused (`refusals`), and whether files can be added now
/// (`canAttach`: not while answering a question). `dragging`: files are dragged over the window.
export function Composer(props: {
  /// What the box holds when it first shows: a draft, not sent until the person sends it.
  initialText?: string;
  disabled: boolean;
  hint: string;
  onSend: (message: string) => Promise<unknown>;
  onSchedule?: (message: string, at: Date) => Promise<unknown>;
  files?: ComposerFile[];
  onAddPaths?: (paths: string[]) => void;
  onAddFiles?: (files: File[]) => void;
  onRemove?: (key: string) => void;
  refusals?: string[];
  canAttach?: boolean;
  dragging?: boolean;
}) {
  const [text, setText] = useState(props.initialText ?? "");
  const files = props.files ?? [];
  const canAttach = (props.canAttach ?? false) && !props.disabled;
  // Read when the picker closes: the box may have become unable to take files meanwhile.
  const canAttachNow = useRef(canAttach);
  useEffect(() => {
    canAttachNow.current = canAttach;
  }, [canAttach]);
  const adding = files.some((f) => f.size === null);
  const hasText = text.trim() !== "";
  // Files count as something to send only while they can be sent: not with an answer.
  const hasContent = hasText || (props.canAttach === true && files.length > 0);
  const canSend = hasContent && !adding && !props.disabled;
  // An idle install never closes Dots over a message being written.
  useComposerReport(hasText);
  const box = useRef<HTMLTextAreaElement>(null);
  // The box is disabled while a send is on its way and while the dot works, and a browser
  // takes the focus from a disabled control. When the box can be used again it gets the focus
  // back, unless the user has put it somewhere else in the meantime.
  const wasDisabled = useRef(props.disabled);
  useEffect(() => {
    const freed = wasDisabled.current && !props.disabled;
    wasDisabled.current = props.disabled;
    const focused = document.activeElement;
    if (freed && (focused === null || focused === document.body || focused === box.current)) {
      box.current?.focus();
    }
  }, [props.disabled]);
  function send() {
    const message = text.trim();
    if (!canSend) return;
    // The mutation's onError reports a failure; here it only keeps the text.
    props.onSend(message).then(
      () => setText(""),
      () => {},
    );
  }
  /// Rejects with the refusal, which the menu shows; the text stays for another try. The
  /// box may have been locked or emptied while the dialog was open: that is a refusal too, so
  /// the dialog does not close as if the message were scheduled.
  function schedule(at: Date): Promise<unknown> {
    const message = text.trim();
    if (props.disabled || !props.onSchedule) return Promise.reject(new Error(SCHEDULE_LOCKED));
    if (!hasContent) return Promise.reject(new Error(SCHEDULE_EMPTY));
    // Files still being added hold Send back, and so they hold this back too.
    if (!canSend) return Promise.reject(new Error(SCHEDULE_LOCKED));
    return props.onSchedule(message, at).then(() => setText(""));
  }
  async function pick() {
    const picked = await open({ multiple: true, directory: false, title: "Attach files" });
    // Picked while the box became disabled or started an answer: not added (section 3.1.11).
    if (picked === null || !canAttachNow.current) return;
    const paths = Array.isArray(picked) ? picked : [picked];
    if (paths.length > 0) props.onAddPaths?.(paths);
  }
  const refusals = props.refusals ?? [];
  return (
    <div className="flex flex-col gap-1.5">
      {props.dragging && (
        <p className="px-1 text-sm text-muted-foreground">Drop files to attach</p>
      )}
      {/* The box draws the focus ring for the text inside it, which has none of its own. */}
      <div
        className={cn(
          "flex items-end gap-2 rounded-2xl border bg-card p-2 shadow-xs focus-within:border-ring focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-ring",
          props.disabled && "cursor-not-allowed opacity-60",
          props.dragging && "border-dashed border-ring",
        )}
      >
        {props.onAddPaths && (
          <Button
            size="icon-sm"
            variant="ghost"
            className="shrink-0 rounded-full"
            aria-label="Attach files"
            title="Attach files"
            disabled={!canAttach}
            onClick={() => void pick()}
          >
            <Paperclip aria-hidden />
          </Button>
        )}
        <label htmlFor="chat-message" className="sr-only">
          Message
        </label>
        <textarea
          id="chat-message"
          ref={box}
          rows={1}
          placeholder={props.hint}
          disabled={props.disabled}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onPaste={(e) => {
            const pasted = Array.from(e.clipboardData?.files ?? []);
            if (pasted.length === 0 || !canAttach || !props.onAddFiles) return;
            e.preventDefault();
            props.onAddFiles(pasted);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              send();
            }
          }}
          className="field-sizing-content max-h-40 min-w-0 flex-1 resize-none overflow-y-auto bg-transparent px-1.5 py-1 text-base outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed md:text-sm"
        />
        {props.onSchedule && <SendLaterMenu disabled={!canSend} onPick={schedule} />}
        <Button
          size="icon-sm"
          className="shrink-0 rounded-full"
          aria-label="Send"
          title="Send"
          disabled={!canSend}
          onClick={send}
        >
          <ArrowUp aria-hidden />
        </Button>
      </div>
      {files.length > 0 && (
        <ul aria-label="Attached files" className="flex flex-wrap gap-1.5 px-1">
          {files.map((f) => (
            <li key={f.key} className="max-w-full min-w-0">
              <AttachmentChip
                name={f.name}
                size={f.size}
                onRemove={
                  f.size === null || !props.onRemove ? undefined : () => props.onRemove?.(f.key)
                }
              />
            </li>
          ))}
        </ul>
      )}
      {refusals.map((r) => (
        <p key={r} role="alert" className="px-1 text-xs text-destructive">
          {r}
        </p>
      ))}
    </div>
  );
}

/// A file in the chat box: `size` null while it is being added.
export type ComposerFile = { key: string; name: string; size: number | null };
