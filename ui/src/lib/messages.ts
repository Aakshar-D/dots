import type { Message } from "@/api/types";

/// A question neither answered nor dismissed.
export function isOpenQuestion(message: Message): boolean {
  return (
    message.kind === "question" && message.answered_at === null && message.dismissed_at === null
  );
}

/// A message that waits for the person: an open question, or a note not read yet. The Inbox
/// shows these and counts them, as the core's `inbox_count` does.
export function waits(message: Message): boolean {
  return message.kind === "question" ? isOpenQuestion(message) : message.read_at === null;
}
