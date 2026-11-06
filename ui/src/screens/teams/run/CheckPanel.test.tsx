import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import type { CheckView, WorkflowRun } from "@/api/types";
import { createQueryClient, keys } from "@/lib/query";
import { backend, later } from "@/test/backend";
import { sample } from "@/test/fixtures";

import { CheckPanel } from "./CheckPanel";

const [waiting, done] = sample.workflowRuns as [WorkflowRun, WorkflowRun];
const LEAD = "dots/nightly-tests/01J0RUN5";
const MEMBER = "dots/reviewer/01J0RUN6";
const TIP = "2222222222222222222222222222222222222222";
/// The step run the sample check waits on, and the next check's.
const STEP_RUN = "01J0STP3";
const NEXT = "01J0STP4";
const CHANGED = `The branches changed since the check was shown: ${LEAD}. Look at the check again.`;

/// The check's actions on their own, as a conversation's workflow card shows them.
function show(run: WorkflowRun, stop?: boolean) {
  backend.on("workflow_check", () => sample.check);
  const queryClient = createQueryClient();
  const view = render(
    <QueryClientProvider client={queryClient}>
      <CheckPanel run={run} stop={stop} />
    </QueryClientProvider>,
  );
  return { ...view, queryClient, user: userEvent.setup() };
}

describe("the check's actions", () => {
  it("show the hand-off's summary, each branch it shows, and Stop unless told not to", async () => {
    show(waiting);
    const check = within(screen.getByRole("region", { name: "Your check" }));
    expect(await check.findByText("Made the icon dark on a light taskbar.")).toBeVisible();
    const branches = within(check.getByRole("list", { name: "Branches to check" }));
    expect(branches.getByRole("button", { name: `View diff of ${LEAD}` })).toBeEnabled();
    const gone = within(branches.getByRole("listitem", { name: MEMBER }));
    expect(gone.getByText("Gone")).toBeVisible();
    expect(gone.getByRole("button", { name: `View diff of ${MEMBER}` })).toBeDisabled();
    expect(gone.getByRole("button", { name: `View diff of ${MEMBER}` })).toHaveAttribute(
      "title",
      "The branch is gone.",
    );
    expect(check.getByRole("button", { name: "Stop" })).toBeVisible();
  });

  it("show nothing while the run does not wait on a check, and leave Stop out when asked", async () => {
    const { container } = show(done);
    expect(container).toBeEmptyDOMElement();
    expect(backend.callsTo("workflow_check")).toEqual([]);
    show(waiting, false);
    await screen.findByRole("listitem", { name: LEAD });
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
  });

  it("send the work back to the check's target only with a note", async () => {
    backend.on("workflow_send_back", () => ({ ...waiting, status: "running", waiting_on: null }));
    const { user, queryClient } = show(waiting);
    const back = screen.getByRole("button", { name: "Send back to Build" });
    expect(back).toBeDisabled();
    expect(back).toHaveAttribute("title", "Write a note first: it says what to change.");
    await user.type(screen.getByRole("textbox", { name: "Note" }), "  Keep the old icon.  ");
    await user.click(back);
    expect(backend.callsTo("workflow_send_back")).toEqual([
      { id: "01J0WFR", note: "Keep the old icon.", stepRunId: STEP_RUN },
    ]);
    await waitFor(() => expect(screen.getByRole("textbox", { name: "Note" })).toHaveValue(""));
    expect(queryClient.getQueryData(keys.workflowRun("01J0WFR"))).toMatchObject({
      status: "running",
    });
  });

  it("approve with the note and the branches and commits the check showed", async () => {
    backend.on("workflow_approve", () => ({ ...done, id: waiting.id }));
    const { user } = show(waiting);
    await screen.findByRole("listitem", { name: LEAD });
    await user.type(screen.getByRole("textbox", { name: "Note" }), "Looks right.");
    await user.click(screen.getByRole("button", { name: "Approve" }));
    expect(backend.callsTo("workflow_approve")).toEqual([
      {
        id: "01J0WFR",
        note: "Looks right.",
        stepRunId: STEP_RUN,
        tips: [
          { branch: LEAD, commit: TIP },
          { branch: MEMBER, commit: null },
        ],
      },
    ]);
  });

  it("name the check on screen by its step run, and wait while the check is read again", async () => {
    backend.on("workflow_approve", () => ({ ...done, id: waiting.id }));
    const { user, queryClient } = show(waiting);
    await screen.findByRole("listitem", { name: LEAD });
    const approve = screen.getByRole("button", { name: "Approve" });
    const back = screen.getByRole("button", { name: "Send back to Build" });
    await user.type(screen.getByRole("textbox", { name: "Note" }), "Looks right.");
    expect(approve).toBeEnabled();
    expect(back).toBeEnabled();
    // The check is read again (its run moved): the same branches, another step run.
    const next = later<CheckView>();
    backend.on("workflow_check", () => next.answer);
    void queryClient.invalidateQueries({ queryKey: keys.workflowCheck(waiting.id) });
    await waitFor(() => expect(approve).toBeDisabled());
    expect(back).toBeDisabled();
    next.give({ ...sample.check, step_run_id: NEXT });
    await waitFor(() => expect(approve).toBeEnabled());
    await user.click(approve);
    expect(backend.callsTo("workflow_approve")).toEqual([
      {
        id: "01J0WFR",
        note: "Looks right.",
        stepRunId: NEXT,
        tips: [
          { branch: LEAD, commit: TIP },
          { branch: MEMBER, commit: null },
        ],
      },
    ]);
  });

  it("drop the check it showed once Approve answers, so the next check is read before Approve is offered", async () => {
    // As on a conversation's card, the run shown still waits on a check after the answer.
    const next = later<CheckView>();
    backend.on("workflow_approve", () => {
      backend.on("workflow_check", () => next.answer);
      return { ...waiting, steps: [...waiting.steps] };
    });
    const { user } = show(waiting);
    await screen.findByRole("listitem", { name: LEAD });
    await user.click(screen.getByRole("button", { name: "Approve" }));
    expect(await screen.findByText("Reading the check…")).toBeVisible();
    expect(screen.queryByRole("listitem", { name: LEAD })).toBeNull();
    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
    expect(backend.callsTo("workflow_approve")).toHaveLength(1);
    next.give({ ...sample.check, step_run_id: NEXT });
    await screen.findByRole("listitem", { name: LEAD });
    await user.click(screen.getByRole("button", { name: "Approve" }));
    expect(backend.callsTo("workflow_approve").map((c) => c.stepRunId)).toEqual([STEP_RUN, NEXT]);
  });

  it("show the core's refusal when a branch moved, read the check again, and approve what it shows now", async () => {
    let answers = 0;
    backend.on("workflow_approve", () => {
      answers += 1;
      if (answers === 1) throw { kind: "conflict", message: CHANGED };
      return { ...done, id: waiting.id };
    });
    const { user } = show(waiting);
    await screen.findByRole("listitem", { name: LEAD });
    const moved = "4444444444444444444444444444444444444444";
    backend.on("workflow_check", () => ({
      ...sample.check,
      branches: [{ ...sample.check.branches[0]!, commit: moved }],
    }));
    await user.click(screen.getByRole("button", { name: "Approve" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(CHANGED);
    await waitFor(() => expect(backend.callsTo("workflow_check")).toHaveLength(2));
    await waitFor(() => expect(screen.queryByRole("listitem", { name: MEMBER })).toBeNull());
    await user.click(screen.getByRole("button", { name: "Approve" }));
    expect(backend.callsTo("workflow_approve")[1]).toEqual({
      id: "01J0WFR",
      note: null,
      stepRunId: STEP_RUN,
      tips: [{ branch: LEAD, commit: moved }],
    });
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  });

  it("show a branch's diff from the default branch in a dialog", async () => {
    backend.on("workflow_branch_diff", () => sample.diff);
    const { user } = show(waiting);
    await user.click(await screen.findByRole("button", { name: `View diff of ${LEAD}` }));
    const dialog = await screen.findByRole("dialog", { name: `Diff of ${LEAD}` });
    expect(dialog).toHaveTextContent(
      "What its commits change, from the project's default branch, at 2222222.",
    );
    expect(await within(dialog).findByText("src/lib.rs")).toBeVisible();
    expect(within(dialog).getByText("logo.png")).toBeVisible();
    expect(backend.callsTo("workflow_branch_diff")).toEqual([
      { id: "01J0WFR", branch: LEAD, commit: TIP },
    ]);
  });
});
