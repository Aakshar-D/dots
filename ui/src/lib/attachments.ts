// Attachments in chat (attachments spec sections 3 and 6): the limits the chat box checks
// before it asks the core, the name of a pasted image, and the files a message carries.
import { z } from "zod";

import { MessageAttachment, type Run } from "@/api/types";

export const MAX_FILES = 10;
export const MAX_FILE_BYTES = 25 * 1024 * 1024;
export const TOO_MANY = "You can attach up to 10 files to a message.";

const PASTED_TYPES: Record<string, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/gif": "gif",
  "image/webp": "webp",
};

/// The name of a pasted file: `Pasted image.<ext>` (from its type, png otherwise), unless the
/// clipboard gave it a name other than the browser's default `image.png`.
export function pastedName(file: { name: string; type: string }): string {
  if (file.name !== "" && file.name !== "image.png") return file.name;
  return `Pasted image.${PASTED_TYPES[file.type] ?? "png"}`;
}

/// How many of `adding` files fit beside `staged` ones, and whether some are refused.
export function room(staged: number, adding: number): { take: number; refused: boolean } {
  const take = Math.max(0, Math.min(adding, MAX_FILES - staged));
  return { take, refused: take < adding };
}

const Listed = z.object({ attachments: z.array(MessageAttachment) });

/// The files a chat message carries (`payload.attachments`); none when absent or unreadable.
export function messageAttachments(run: Pick<Run, "payload">): MessageAttachment[] {
  const parsed = Listed.safeParse(run.payload);
  return parsed.success ? parsed.data.attachments : [];
}
