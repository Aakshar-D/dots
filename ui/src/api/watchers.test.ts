import { describe, expect, it } from "vitest";

import { commands } from "@/api/commands";
import type { Watcher } from "@/api/types";
import { createQueryClient, keys } from "@/lib/query";
import { applyRuntimeEvent } from "@/lib/runtime-events";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";

type Client = ReturnType<typeof createQueryClient>;
const stale = (qc: Client, key: readonly unknown[]) => qc.getQueryState(key)?.isInvalidated;

describe("the watcher commands", () => {
  it("list, add, change, switch, resume and delete as the core names them", async () => {
    const [folder, email] = sample.watchers as [Watcher, Watcher];
    backend.on("watchers_list", () => sample.watchers);
    backend.on("watcher_add", () => folder);
    backend.on("watcher_update", () => folder);
    backend.on("watcher_set_enabled", () => folder);
    backend.on("watcher_resume", () => email);
    backend.on("watcher_delete", () => null);
    expect(await commands.watchersList("dot-a")).toEqual(sample.watchers);
    expect(await commands.watcherAdd("dot-a", "Invoices folder", folder.config)).toEqual(folder);
    await commands.watcherUpdate(folder.id, "Bills", folder.config);
    await commands.watcherSetEnabled(folder.id, false);
    expect(await commands.watcherResume(email.id)).toEqual(email);
    expect(await commands.watcherDelete(folder.id)).toBeNull();
    expect(backend.calls).toEqual([
      ["watchers_list", { dotId: "dot-a" }],
      ["watcher_add", { dotId: "dot-a", name: "Invoices folder", config: folder.config }],
      ["watcher_update", { id: folder.id, name: "Bills", config: folder.config }],
      ["watcher_set_enabled", { id: folder.id, enabled: false }],
      ["watcher_resume", { id: email.id }],
      ["watcher_delete", { id: folder.id }],
    ]);
  });

  it("read every kind and state of watcher, and the paused ones of the Inbox", () => {
    expect(sample.watchers.map((w) => [w.config.kind, w.paused_reason])).toEqual([
      ["folder", null],
      ["email", "error"],
      ["github", "flood"],
    ]);
    expect(sample.inbox.watchers.map((w) => [w.name, w.reason, w.dot_name])).toEqual([
      ["Billing mail", "error", "nightly-tests"],
    ]);
  });

  it("refresh the dot's watchers, the Inbox and the dots' counts when a watcher changes", () => {
    const qc = createQueryClient();
    for (const key of [keys.watchers("dot-a"), keys.watchers("dot-b"), keys.inbox, keys.overview]) {
      qc.setQueryData(key, []);
    }
    applyRuntimeEvent(qc, { type: "watcher_updated", dot_id: "dot-a", watcher_id: "w1" });
    expect(stale(qc, keys.watchers("dot-a"))).toBe(true);
    expect(stale(qc, keys.watchers("dot-b"))).toBe(false);
    expect(stale(qc, keys.inbox)).toBe(true);
    expect(stale(qc, keys.overview)).toBe(true);

    const quiet = createQueryClient();
    quiet.setQueryData(keys.inbox, []);
    applyRuntimeEvent(quiet, {
      type: "watcher_paused",
      dot_id: "dot-a",
      watcher_id: "w1",
      name: "Invoices folder",
      reason: "flood",
      error: null,
    });
    expect(stale(quiet, keys.inbox)).toBe(false);
  });
});
