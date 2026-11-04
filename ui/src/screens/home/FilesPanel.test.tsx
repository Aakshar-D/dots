import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { ConversationFile } from "@/api/types";
import { backend } from "@/test/backend";
import { chat, dot, home } from "@/test/home";
import { renderApp } from "@/test/render";

const file = (over: Partial<ConversationFile>): ConversationFile => ({
  path: "C:\\work\\attachments\\a\\r.pdf",
  name: "r.pdf",
  source: "attached",
  run_id: "a",
  at: "2026-10-09T10:05:00.000Z",
  exists: true,
  can_open: true,
  ...over,
});
const report = file({});
const written = file({
  path: "C:\\work\\out.md",
  name: "out.md",
  source: "written",
  at: "2026-10-09T10:01:00.000Z",
});

function withFiles(files: ConversationFile[]) {
  home([chat("a", "10:00")]);
  backend.on("conversation_files", () => files);
  backend.on("conversation_file_open", () => null);
  backend.on("conversation_file_reveal", () => null);
}

const panel = () => screen.findByRole("complementary", { name: "Files" });

describe("the Files panel", () => {
  it("lists attached and written files, newest first", async () => {
    withFiles([report, written]);
    renderApp(`/home/${dot.id}`);
    const rows = within(await panel()).getAllByRole("listitem");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent("r.pdf");
    expect(rows[0]).toHaveTextContent(/Attached · /);
    expect(rows[1]).toHaveTextContent("out.md");
    expect(rows[1]).toHaveTextContent(`Changed by ${dot.name} · `);
    expect(backend.callsTo("conversation_files")[0]).toEqual({
      dotId: dot.id,
      rootRunIds: ["a"],
    });
  });

  it("opens a file on a click and shows it in its folder", async () => {
    withFiles([report]);
    const { user } = renderApp(`/home/${dot.id}`);
    const view = within(await panel());
    const name = view.getByRole("button", { name: "r.pdf" });
    expect(name).toHaveAttribute("title", report.path);
    await user.click(name);
    expect(backend.callsTo("conversation_file_open")).toEqual([
      { dotId: dot.id, rootRunIds: ["a"], path: report.path },
    ]);
    await user.click(view.getByRole("button", { name: "Show r.pdf in folder" }));
    expect(backend.callsTo("conversation_file_reveal")).toEqual([
      { dotId: dot.id, rootRunIds: ["a"], path: report.path },
    ]);
  });

  it("says why an open failed", async () => {
    withFiles([report]);
    backend.on("conversation_file_open", () => {
      throw { kind: "not_found", message: "this file is no longer there" };
    });
    const { user } = renderApp(`/home/${dot.id}`);
    await user.click(within(await panel()).getByRole("button", { name: "r.pdf" }));
    expect(await screen.findByText("this file is no longer there")).toBeVisible();
  });

  it("a gone file has no actions", async () => {
    withFiles([file({ ...written, exists: false })]);
    renderApp(`/home/${dot.id}`);
    const view = within(await panel());
    expect(view.getByText("Gone")).toBeVisible();
    expect(view.queryByRole("button", { name: "out.md" })).toBeNull();
    expect(view.queryByRole("button", { name: "Show out.md in folder" })).toBeNull();
  });

  it("a file Dots doesn't open is not a button, says why, and can be shown in its folder", async () => {
    const why = "Dots only opens documents, images and media. Use Show in folder.";
    withFiles([file({ path: "C:\\work\\run.ps1", name: "run.ps1", can_open: false })]);
    renderApp(`/home/${dot.id}`);
    const view = within(await panel());
    expect(view.queryByRole("button", { name: "run.ps1" })).toBeNull();
    expect(view.getByText("run.ps1")).toHaveAttribute("title", why);
    // Said in the open, and read out with the folder button: not only in a tooltip.
    expect(view.getByText(why)).toBeVisible();
    expect(
      view.getByRole("button", { name: "Show run.ps1 in folder" }),
    ).toHaveAccessibleDescription(why);
  });

  it("hides with the toggle and remembers it", async () => {
    withFiles([report]);
    const first = renderApp(`/home/${dot.id}`);
    await panel();
    const toggle = screen.getByRole("button", { name: "Files" });
    expect(toggle).toHaveAttribute("aria-pressed", "true");
    expect(toggle).toHaveAttribute("title", "Hide files");
    await first.user.click(toggle);
    expect(screen.queryByRole("complementary", { name: "Files" })).toBeNull();
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    expect(toggle).toHaveAttribute("title", "Show files");
    expect(window.localStorage.getItem("dots.home.files")).toBe("hidden");
    first.unmount();

    const again = renderApp(`/home/${dot.id}`);
    const shown = await screen.findByRole("button", { name: "Files" });
    expect(shown).toHaveAttribute("aria-pressed", "false");
    expect(screen.queryByRole("complementary", { name: "Files" })).toBeNull();
    await again.user.click(shown);
    expect(await panel()).toBeInTheDocument();
    expect(window.localStorage.getItem("dots.home.files")).toBeNull();
  });

  it("shows neither panel nor toggle without files", async () => {
    withFiles([]);
    renderApp(`/home/${dot.id}`);
    await screen.findByRole("region", { name: "Conversation" });
    await waitFor(() => expect(backend.callsTo("conversation_files")).toHaveLength(1));
    expect(screen.queryByRole("complementary", { name: "Files" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Files" })).toBeNull();
  });

  it("is loaded again when a run changes", async () => {
    withFiles([report]);
    const { listening } = renderApp(`/home/${dot.id}`);
    await panel();
    await listening();
    const before = backend.callsTo("conversation_files").length;
    act(() =>
      backend.emit("runtime", { type: "run_updated", run: chat("a", "10:00") }),
    );
    await waitFor(() =>
      expect(backend.callsTo("conversation_files").length).toBeGreaterThan(before),
    );
  });
});
