import { Send } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { reportComposer } from "@/lib/use-activity";

/// Where a message is typed. Enter sends; Shift+Enter starts a new line. The text is cleared
/// once the send has succeeded; a refused message stays for another try.
export function Composer(props: {
  disabled: boolean;
  hint: string;
  onSend: (message: string) => Promise<unknown>;
}) {
  const [text, setText] = useState("");
  const hasText = text.trim() !== "";
  // An idle install never closes Dots over a message being written.
  useEffect(() => reportComposer(hasText), [hasText]);
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
  return (
    <div className="flex items-end gap-2 px-3 pb-3">
      <label htmlFor="chat-message" className="sr-only">
        Message
      </label>
      <Textarea
        id="chat-message"
        ref={box}
        rows={2}
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
      />
      <Button disabled={props.disabled || text.trim() === ""} onClick={send}>
        <Send aria-hidden /> Send
      </Button>
    </div>
  );
}
