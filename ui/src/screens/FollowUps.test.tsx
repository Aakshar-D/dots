import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { FollowUp } from "@/api/types";
import { localDateTime } from "@/lib/format";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [pending] = sample.followUps;

describe("follow-ups", () => {
  it("lists a dot's pending follow-ups in its editor and cancels one", async () => {
    let list: FollowUp[] = [{ ...pending, dot_id: sample.dot.id }];
    backend.on("get_dot", () => sample.dot);
    backend.on("follow_ups_list", (args) => (args.dotId === sample.dot.id ? list : []));
    backend.on("follow_up_cancel", (args) => {
      list = [];
      return { ...pending, id: String(args.id), status: "cancelled" };
    });
    const { user } = renderApp(`/dots/${sample.dot.id}`);
    await user.click(await screen.findByRole("tab", { name: "Follow-ups" }));
    const item = await screen.findByRole("listitem", {
      name: `Follow-up due ${localDateTime(pending.due_at)}`,
    });
    expect(item).toHaveTextContent(pending.note);
    await user.click(within(item).getByRole("button", { name: "Cancel" }));
    await waitFor(() =>
      expect(backend.callsTo("follow_up_cancel")).toEqual([{ id: pending.id }]),
    );
    expect(await screen.findByText(/No follow-up waits/)).toBeVisible();
  });

  it("says in the Dots list when a dot's next follow-up fires", async () => {
    backend.on("dot_overview", () => [sample.overview]);
    const soon = new Date(Date.now() + 3 * 3_600_000).toISOString();
    backend.on("follow_ups_list", () => [
      { ...pending, dot_id: sample.overview.dot.id, due_at: soon },
    ]);
    renderApp("/dots");
    expect(await screen.findByText("Next follow-up in 3 h")).toBeVisible();
  });
});
