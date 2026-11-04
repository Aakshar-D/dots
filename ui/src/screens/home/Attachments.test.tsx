import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { StagedAttachment } from "@/api/types";
import { backend, later } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { chat, dot, home } from "@/test/home";
import { renderApp } from "@/test/render";

const box = () => screen.getByLabelText("Message");
async function readyBox() {
  const found = await screen.findByLabelText("Message");
  await waitFor(() => expect(found).toBeEnabled());
  return found;
}
const send = () => screen.getByRole("button", { name: "Send" });
const chips = () => screen.queryByRole("list", { name: "Attached files" });

/// The core stages each path as a file named after it, 2458 bytes, with ids `S1`, `S2`…
function staging() {
  let n = 0;
  backend.on(
    "attachment_add_path",
    (args): StagedAttachment => ({
      id: `S${++n}`,
      name: String(args.path).split(/[\\/]/).pop()!,
      size: 2458,
    }),
  );
  backend.on("attachment_add_bytes", (): StagedAttachment => ({
    id: `S${++n}`,
    name: "Pasted image.png",
    size: 10,
  }));
  backend.on("attachment_remove", () => null);
}

function drop(paths: string[]) {
  act(() => backend.emit("drag-drop", { type: "drop", paths, position: { x: 0, y: 0 } }));
}

describe("attaching files to a message", () => {
  it("the paperclip attaches picked files", async () => {
    home([chat("a", "10:00")]);
    staging();
    backend.on("dialog_open", () => ["C:\\docs\\report.pdf", "C:\\docs\\notes.txt"]);
    const { user } = renderApp(`/home/${dot.id}`);
    await readyBox();
    await user.click(screen.getByRole("button", { name: "Attach files" }));
    expect(backend.callsTo("dialog_open")).toEqual([
      { multiple: true, directory: false, title: "Attach files" },
    ]);
    await waitFor(() =>
      expect(backend.callsTo("attachment_add_path")).toEqual([
        { dotId: dot.id, path: "C:\\docs\\report.pdf" },
        { dotId: dot.id, path: "C:\\docs\\notes.txt" },
      ]),
    );
    const list = within(chips()!);
    expect(await list.findByText("report.pdf")).toBeVisible();
    expect(list.getByText("notes.txt")).toBeVisible();
    expect(list.getAllByText("2.4 KB")).toHaveLength(2);
  });

  it("says why a file was refused, and the others stay", async () => {
    home([chat("a", "10:00")]);
    staging();
    backend.on("attachment_add_path", (args) => {
      if (String(args.path).endsWith("big.bin")) {
        throw { kind: "invalid", message: "big.bin is larger than 25 MB." };
      }
      return { id: "S1", name: "ok.txt", size: 3 };
    });
    renderApp(`/home/${dot.id}`);
    await readyBox();
    drop(["C:\\ok.txt", "C:\\big.bin"]);
    expect(await screen.findByRole("alert")).toHaveTextContent("big.bin is larger than 25 MB.");
    await waitFor(() => expect(within(chips()!).getAllByRole("listitem")).toHaveLength(1));
    expect(within(chips()!).getByText("ok.txt")).toBeVisible();
  });

  it("refuses an eleventh file without asking the core", async () => {
    home([chat("a", "10:00")]);
    staging();
    renderApp(`/home/${dot.id}`);
    await readyBox();
    drop(Array.from({ length: 10 }, (_, i) => `C:\\f${i}.txt`));
    await waitFor(() => expect(within(chips()!).getAllByText("2.4 KB")).toHaveLength(10));
    drop(["C:\\eleven.txt"]);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "You can attach up to 10 files to a message.",
    );
    expect(backend.callsTo("attachment_add_path")).toHaveLength(10);
  });

  it("attaches a drop, and says so while files are dragged", async () => {
    home([chat("a", "10:00")]);
    staging();
    renderApp(`/home/${dot.id}`);
    await readyBox();
    act(() =>
      backend.emit("drag-drop", { type: "enter", paths: [], position: { x: 0, y: 0 } }),
    );
    expect(await screen.findByText("Drop files to attach")).toBeVisible();
    drop(["C:\\a.txt"]);
    await waitFor(() =>
      expect(backend.callsTo("attachment_add_path")).toEqual([
        { dotId: dot.id, path: "C:\\a.txt" },
      ]),
    );
    expect(screen.queryByText("Drop files to attach")).toBeNull();
  });

  it("sends a pasted screenshot as bytes", async () => {
    home([chat("a", "10:00")]);
    staging();
    renderApp(`/home/${dot.id}`);
    const input = await readyBox();
    const file = new File([new Uint8Array([1, 2, 3])], "image.png", { type: "image/png" });
    fireEvent.paste(input, { clipboardData: { files: [file], getData: () => "" } });
    await waitFor(() => expect(backend.callsTo("attachment_add_bytes")).toHaveLength(1));
    const [call] = backend.callsTo("attachment_add_bytes");
    expect(call["x-dots-name"]).toBe("Pasted%20image.png");
    expect(call["x-dots-dot"]).toBe(dot.id);
    expect(Array.from(call.bytes as Uint8Array)).toEqual([1, 2, 3]);
    expect(await within(chips()!).findByText("Pasted image.png")).toBeVisible();
  });

  it("removing a chip unstages its file", async () => {
    home([chat("a", "10:00")]);
    staging();
    const { user } = renderApp(`/home/${dot.id}`);
    await readyBox();
    drop(["C:\\report.pdf"]);
    await user.click(await screen.findByRole("button", { name: "Remove report.pdf" }));
    expect(backend.callsTo("attachment_remove")).toEqual([{ id: "S1" }]);
    expect(chips()).toBeNull();
  });

  it("send passes the files and clears them", async () => {
    home([chat("a", "10:00")]);
    staging();
    const { user } = renderApp(`/home/${dot.id}`);
    await readyBox();
    drop(["C:\\one.txt", "C:\\two.txt"]);
    await waitFor(() => expect(within(chips()!).getAllByText("2.4 KB")).toHaveLength(2));
    await user.type(box(), "Shorten it");
    await user.click(send());
    await waitFor(() =>
      expect(backend.callsTo("chat")).toEqual([
        { dotId: dot.id, message: "Shorten it", parentRunId: "a", attachmentIds: ["S1", "S2"] },
      ]),
    );
    await waitFor(() => expect(chips()).toBeNull());
    // Leaving the conversation removes nothing that was sent.
    expect(backend.callsTo("attachment_remove")).toEqual([]);
  });

  it("a message may be files only", async () => {
    home([chat("a", "10:00")]);
    staging();
    const { user } = renderApp(`/home/${dot.id}`);
    await readyBox();
    expect(send()).toBeDisabled();
    drop(["C:\\one.txt"]);
    await waitFor(() => expect(send()).toBeEnabled());
    await user.click(send());
    await waitFor(() =>
      expect(backend.callsTo("chat")).toEqual([
        { dotId: dot.id, message: "", parentRunId: "a", attachmentIds: ["S1"] },
      ]),
    );
  });

  it("send waits for files being added", async () => {
    home([chat("a", "10:00")]);
    const pending = later<StagedAttachment>();
    backend.on("attachment_add_path", () => pending.answer);
    const { user } = renderApp(`/home/${dot.id}`);
    await readyBox();
    drop(["C:\\slow.bin"]);
    expect(await within(chips()!).findByText("Adding…")).toBeVisible();
    await user.type(box(), "Here");
    expect(send()).toBeDisabled();
    await act(async () => pending.give({ id: "S1", name: "slow.bin", size: 5 }));
    await waitFor(() => expect(send()).toBeEnabled());
  });

  it("an answer to a question takes no files", async () => {
    home([chat("a", "10:00")]);
    staging();
    backend.on("messages_list", () => [{ ...sample.messages[0], run_id: "a" }]);
    const { user } = renderApp(`/home/${dot.id}`);
    const bar = await screen.findByRole("region", { name: "Open question" });
    await readyBox();
    await user.click(within(bar).getByRole("button", { name: "Reply to question" }));
    expect(screen.getByRole("button", { name: "Attach files" })).toBeDisabled();
    drop(["C:\\a.txt"]);
    expect(backend.callsTo("attachment_add_path")).toEqual([]);
  });

  it("files left in the box are removed when the conversation goes away", async () => {
    home([chat("a", "10:00")]);
    staging();
    const { unmount } = renderApp(`/home/${dot.id}`);
    await readyBox();
    drop(["C:\\left.txt"]);
    await waitFor(() => expect(within(chips()!).getByText("2.4 KB")).toBeVisible());
    unmount();
    await waitFor(() => expect(backend.callsTo("attachment_remove")).toEqual([{ id: "S1" }]));
  });
});

describe("a message's files in the conversation", () => {
  it("shows them under the bubble, a files-only message included", async () => {
    home([
      chat("a", "10:00", {
        payload: { message: "", typed: true, attachments: [{ id: "A1", name: "r.pdf", size: 2458 }] },
      }),
    ]);
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    const list = within(await view.findByRole("list", { name: "Attached files" }));
    expect(list.getByText("r.pdf")).toBeVisible();
    expect(list.getByText("2.4 KB")).toBeVisible();
    expect(list.queryByRole("button")).toBeNull();
  });
});
