import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { TopicInfo } from "@/api/types";
import { backend } from "@/test/backend";
import { chat, dot, home, reply } from "@/test/home";
import { renderApp } from "@/test/render";

const row = (root_run_id: string, over: Partial<TopicInfo> = {}): TopicInfo => ({
  root_run_id,
  dot_id: dot.id,
  title: null,
  archived_at: null,
  ...over,
});

async function ready() {
  const box = await screen.findByLabelText("Message");
  await waitFor(() => expect(box).toBeEnabled());
}

describe("topic tools on the home", () => {
  it("names topics as the person did and lists archived ones apart in History", async () => {
    home([chat("old", "08:00"), chat("gone", "09:00"), chat("current", "10:00")]);
    backend.on("list_topics", () => [
      row("old", { title: "Trip plans" }),
      row("gone", { archived_at: "2026-10-09T12:00:00Z" }),
    ]);
    const { user } = renderApp(`/home/${dot.id}`);
    await ready();
    await user.click(screen.getByRole("button", { name: "History" }));
    const items = await screen.findAllByRole("menuitem");
    expect(items.map((item) => item.textContent)).toEqual([
      expect.stringMatching(/^Trip plans/),
      expect.stringMatching(/^question gone/),
    ]);
    const archived = screen.getByRole("group", { name: "Archived" });
    expect(within(archived).getByRole("menuitem")).toHaveTextContent("question gone");
  });

  it("renames, archives and merges the topic on screen from its menu", async () => {
    home([chat("old", "08:00"), chat("current", "10:00")]);
    backend.on("list_topics", () => []);
    backend.on("rename_topic", () => row("current", { title: "Sink" }));
    backend.on("set_topic_archived", () => row("current", { archived_at: "2026-10-09T12:00:00Z" }));
    backend.on("merge_topics", () => null);
    const { user, router } = renderApp(`/home/${dot.id}`);
    await ready();

    await user.click(screen.getByRole("button", { name: "Topic" }));
    await user.click(await screen.findByRole("menuitem", { name: "Rename…" }));
    await user.type(screen.getByLabelText("Name"), "Sink");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("rename_topic")).toEqual([{ rootRunId: "current", title: "Sink" }]),
    );

    await user.click(screen.getByRole("button", { name: "Topic" }));
    await user.click(await screen.findByRole("menuitem", { name: "Archive" }));
    await waitFor(() =>
      expect(backend.callsTo("set_topic_archived")).toEqual([
        { rootRunId: "current", archived: true },
      ]),
    );

    await user.click(screen.getByRole("button", { name: "Topic" }));
    await user.click(await screen.findByRole("menuitem", { name: "Merge into…" }));
    await user.selectOptions(screen.getByLabelText("Topic to merge into"), "old");
    await user.click(screen.getByRole("button", { name: "Merge" }));
    await waitFor(() =>
      expect(backend.callsTo("merge_topics")).toEqual([
        { fromRootRunId: "current", intoRootRunId: "old" },
      ]),
    );
    await waitFor(() => expect(router.state.location.pathname).toBe(`/home/${dot.id}/old`));
  });

  it("moves a message the person sent to another topic or a new one", async () => {
    const first = chat("current", "10:00");
    home([chat("old", "08:00"), first, reply("second", first, "10:05")]);
    backend.on("list_topics", () => []);
    backend.on("move_run", (args) => (args.toRootRunId as string | null) ?? "second");
    const { user } = renderApp(`/home/${dot.id}`);
    await ready();
    // Only on a message that does not start its topic.
    const moves = await screen.findAllByRole("button", { name: "Move to topic…" });
    expect(moves).toHaveLength(1);

    await user.click(moves[0]);
    await user.click(screen.getByRole("button", { name: "Move" }));
    await user.click(await screen.findByRole("button", { name: "Move to topic…" }));
    await user.selectOptions(screen.getByLabelText("Topic to move to"), "old");
    await user.click(screen.getByRole("button", { name: "Move" }));
    await waitFor(() =>
      expect(backend.callsTo("move_run")).toEqual([
        { runId: "second", toRootRunId: null },
        { runId: "second", toRootRunId: "old" },
      ]),
    );
  });

  it("shows the core's refusal in the dialog", async () => {
    const first = chat("current", "10:00");
    home([chat("old", "08:00"), first, reply("second", first, "10:05")]);
    backend.on("list_topics", () => []);
    backend.on("move_run", () => {
      throw { kind: "conflict", message: "a topic that is still working cannot be changed" };
    });
    const { user } = renderApp(`/home/${dot.id}`);
    await ready();
    await user.click(await screen.findByRole("button", { name: "Move to topic…" }));
    await user.click(screen.getByRole("button", { name: "Move" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("still working");
    expect(screen.getByRole("dialog")).toBeVisible();
  });
});
