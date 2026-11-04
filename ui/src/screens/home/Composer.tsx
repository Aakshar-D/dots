import { cn } from "cn";
import { ArrowUp } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { useComposerReport } from "@/lib/use-activity";
import { SendLaterMenu } from "@/screens/home/SendLaterMenu";

/// Where a message is typed: one rounded box holding the text and, at its bottom right, the send
/// button (conversational chat spec section 5). The text grows with what is typed, up to a few
/// lines, then scrolls. Enter sends; Shift+Enter starts a new line. The text is cleared once the
/// send has succeeded; a refused message stays for another try. With `onSchedule`, an arrow
/// beside Send schedules the text for later instead (send later spec section 6), usable
/// exactly when Send is; the text is cleared once it is scheduled.
export function Composer(props: {
  disabled: boolean;
  hint: string;
  onSend: (message: string) => Promise<unknown>;
  onSchedule?: (message: string, at: Date) => Promise<unknown>;
}) {
  const [text, setText] = useState("");
  const hasText = text.trim() !== "";
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
    if (message === "" || props.disabled) return;
    // The mutation's onError reports a failure; here it only keeps the text.
    props.onSend(message).then(
      () => setText(""),
      () => {},
    );
  }
  /// Rejects with the refusal, which the menu shows; the text stays for another try.
  function schedule(at: Date): Promise<unknown> {
    const message = text.trim();
    if (message === "" || props.disabled || !props.onSchedule) return Promise.resolve();
    return props.onSchedule(message, at).then(() => setText(""));
  }
  return (
    // The box draws the focus ring for the text inside it, which has none of its own.
    <div
      className={cn(
        "flex items-end gap-2 rounded-2xl border bg-card p-2 shadow-xs focus-within:border-ring focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-ring",
        props.disabled && "cursor-not-allowed opacity-60",
      )}
    >
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
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            send();
          }
        }}
        className="field-sizing-content max-h-40 min-w-0 flex-1 resize-none overflow-y-auto bg-transparent px-1.5 py-1 text-base outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed md:text-sm"
      />
      {props.onSchedule && (
        <SendLaterMenu disabled={props.disabled || !hasText} onPick={schedule} />
      )}
      <Button
        size="icon-sm"
        className="shrink-0 rounded-full"
        aria-label="Send"
        title="Send"
        disabled={props.disabled || !hasText}
        onClick={send}
      >
        <ArrowUp aria-hidden />
      </Button>
    </div>
  );
}
