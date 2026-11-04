import { describe, expect, it } from "vitest";

import type { Run } from "@/api/types";
import { messageAttachments, pastedName, room } from "@/lib/attachments";

describe("attachments", () => {
  it("names a pasted image after its type unless it has a name of its own", () => {
    expect(pastedName({ name: "image.png", type: "image/png" })).toBe("Pasted image.png");
    expect(pastedName({ name: "", type: "image/jpeg" })).toBe("Pasted image.jpg");
    expect(pastedName({ name: "", type: "image/gif" })).toBe("Pasted image.gif");
    expect(pastedName({ name: "diagram.webp", type: "image/webp" })).toBe("diagram.webp");
    expect(pastedName({ name: "", type: "application/x" })).toBe("Pasted image.png");
  });

  it("takes as many files as there is room for", () => {
    expect(room(8, 3)).toEqual({ take: 2, refused: true });
    expect(room(0, 10)).toEqual({ take: 10, refused: false });
    expect(room(10, 1)).toEqual({ take: 0, refused: true });
  });

  it("reads a message's files from its payload", () => {
    const run = (payload: unknown) => ({ payload }) as unknown as Run;
    expect(messageAttachments(run({ message: "hi" }))).toEqual([]);
    expect(messageAttachments(run(null))).toEqual([]);
    expect(messageAttachments(run({ attachments: "nope" }))).toEqual([]);
    expect(
      messageAttachments(run({ attachments: [{ id: "A1", name: "r.pdf", size: 2458 }] })),
    ).toEqual([{ id: "A1", name: "r.pdf", size: 2458 }]);
  });
});
