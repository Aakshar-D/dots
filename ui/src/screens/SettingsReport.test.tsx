import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const REPORT = "Sage problem report\n\n== What went wrong ==\n(not described)\n";

/// Settings, with a report that names what it was asked for.
function settings() {
  backend.on("get_settings", () => sample.settings);
  backend.on("problem_report", (args) =>
    args.includeLogs
      ? `${REPORT}${String(args.description)}\n== Recent log lines (1, scrubbed) ==\nstarting Sage\n`
      : `${REPORT}${String(args.description)}\n== Recent log lines ==\n(not included)\n`,
  );
  return renderApp("/settings/help");
}

async function openDialog(user: ReturnType<typeof settings>["user"]) {
  const help = await screen.findByRole("group", { name: "Help" });
  await user.click(within(help).getByRole("button", { name: "Report a problem" }));
  return screen.findByRole("dialog", { name: "Report a problem" });
}

const preview = (dialog: HTMLElement) => within(dialog).getByLabelText("What will be shared");

describe("report a problem", () => {
  it("opens from Settings and previews the report with log lines", async () => {
    const { user } = settings();
    const dialog = await openDialog(user);
    expect(within(dialog).getByLabelText("Include recent log lines")).toBeChecked();
    await waitFor(() => expect(preview(dialog)).toHaveTextContent("starting Sage"));
    expect(backend.callsTo("problem_report")[0]).toEqual({ description: "", includeLogs: true });
  });

  it("rebuilds the preview from the text and the checkbox", async () => {
    const { user } = settings();
    const dialog = await openDialog(user);
    await user.type(within(dialog).getByLabelText("What went wrong?"), "It froze");
    await waitFor(() => expect(preview(dialog)).toHaveTextContent("It froze"));
    await user.click(within(dialog).getByLabelText("Include recent log lines"));
    await waitFor(() => expect(preview(dialog)).toHaveTextContent("(not included)"));
    expect(preview(dialog)).not.toHaveTextContent("starting Sage");
    expect(backend.callsTo("problem_report").at(-1)).toEqual({
      description: "It froze",
      includeLogs: false,
    });
  });

  it("copies exactly the previewed text", async () => {
    const { user } = settings();
    const dialog = await openDialog(user);
    await waitFor(() => expect(preview(dialog)).toHaveTextContent("starting Sage"));
    const shown = (preview(dialog) as HTMLTextAreaElement).value;
    await user.click(within(dialog).getByRole("button", { name: "Copy" }));
    expect(await navigator.clipboard.readText()).toBe(shown);
    expect(await screen.findByText("Copied the report")).toBeVisible();
  });

  it("saves the previewed text through the save dialog", async () => {
    const { user } = settings();
    backend.on("save_problem_report", () => "C:/Users/u/Downloads/dots-report-20261103-1405.txt");
    const dialog = await openDialog(user);
    await waitFor(() => expect(preview(dialog)).toHaveTextContent("starting Sage"));
    const shown = (preview(dialog) as HTMLTextAreaElement).value;
    await user.click(within(dialog).getByRole("button", { name: "Save to file…" }));
    expect(
      await screen.findByText("Saved to C:/Users/u/Downloads/dots-report-20261103-1405.txt"),
    ).toBeVisible();
    expect(backend.callsTo("save_problem_report")).toEqual([{ text: shown }]);
  });

  it("says nothing when the save dialog is cancelled", async () => {
    const { user } = settings();
    backend.on("save_problem_report", () => null);
    const dialog = await openDialog(user);
    await waitFor(() => expect(preview(dialog)).toHaveTextContent("starting Sage"));
    await user.click(within(dialog).getByRole("button", { name: "Save to file…" }));
    await waitFor(() => expect(backend.callsTo("save_problem_report")).toHaveLength(1));
    expect(screen.queryByText(/^Saved to/)).toBeNull();
  });

  it("cannot copy or save an old preview while a new one is being made", async () => {
    const { user } = settings();
    let release: () => void = () => {};
    const dialog = await openDialog(user);
    await waitFor(() => expect(preview(dialog)).toHaveTextContent("starting Sage"));
    const copy = within(dialog).getByRole("button", { name: "Copy" });
    const save = within(dialog).getByRole("button", { name: "Save to file…" });
    expect(copy).toBeEnabled();

    // Typing: the preview still shows the old text until the new one comes.
    await user.type(within(dialog).getByLabelText("What went wrong?"), "x");
    expect(copy).toBeDisabled();
    expect(save).toBeDisabled();
    await waitFor(() => expect(preview(dialog)).toHaveTextContent("x"));
    await waitFor(() => expect(copy).toBeEnabled());

    // Unticking the box: disabled until the report without log lines arrives.
    backend.on(
      "problem_report",
      () =>
        new Promise<string>((resolve) => {
          release = () => resolve(`${REPORT}x\n== Recent log lines ==\n(not included)\n`);
        }),
    );
    await user.click(within(dialog).getByLabelText("Include recent log lines"));
    await waitFor(() =>
      expect(backend.callsTo("problem_report").at(-1)).toMatchObject({ includeLogs: false }),
    );
    expect(copy).toBeDisabled();
    expect(save).toBeDisabled();
    release();
    await waitFor(() => expect(preview(dialog)).toHaveTextContent("(not included)"));
    await waitFor(() => expect(copy).toBeEnabled());
  });
});
