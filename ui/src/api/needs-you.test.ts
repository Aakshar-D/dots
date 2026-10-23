import { describe, expect, it } from "vitest";

import { commands } from "@/api/commands";
import { backend } from "@/test/backend";

describe("the commands of what needs the person", () => {
  it("marks a dot seen, says which dot is viewed, and reads the total unread", async () => {
    backend.on("unread_total", () => 4);
    await commands.markDotSeen("dot-a");
    await commands.setViewing("dot-a");
    await commands.setViewing(null);
    expect(await commands.unreadTotal()).toBe(4);
    expect(backend.calls).toEqual([
      ["mark_dot_seen", { dotId: "dot-a" }],
      ["set_viewing", { dotId: "dot-a" }],
      ["set_viewing", { dotId: null }],
      ["unread_total", {}],
    ]);
  });
});
