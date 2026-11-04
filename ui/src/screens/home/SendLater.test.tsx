import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { ScheduledMessage } from "@/api/types";
import { toPicker, type LaterPreset } from "@/lib/later";
import { Composer, SCHEDULE_LOCKED } from "@/screens/home/Composer";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { chat, dot, home, reply } from "@/test/home";
import { renderApp } from "@/test/render";

/// The one preset these tests offer: in an hour, at a time fixed when the tests load.
const AT = new Date(Date.now() + 3_600_000);
/// Other presets a test offers instead, given the time the menu opens at.
const offer = vi.hoisted(() => ({ presets: null as null | ((now: Date) => LaterPreset[]) }));
vi.mock("@/lib/later", async (actual) => ({
  ...(await actual<typeof import("@/lib/later")>()),
  laterPresets: (now: Date) =>
    offer.presets?.(now) ?? [{ id: "hour", label: "In 1 hour", at: AT }],
}));

afterEach(() => {
  offer.presets = null;
});

const box = () => screen.getByLabelText("Message");
async function readyBox() {
  const found = await screen.findByLabelText("Message");
  await waitFor(() => expect(found).toBeEnabled());
  return found;
}

/// The core's answer to `schedule_message`.
function scheduling() {
  backend.on(
    "schedule_message",
    (args): ScheduledMessage => ({
      ...sample.scheduledMessage,
      dot_id: args.dotId as string,
      root_run_id: args.rootRunId as string | null,
      text: args.text as string,
      due_at: args.dueAt as string,
      waiting: null,
    }),
  );
}

async function openMenu(user: ReturnType<typeof renderApp>["user"]) {
  await user.click(screen.getByRole("button", { name: "Send later" }));
  return screen.findByRole("menu");
}

describe("the Send later menu", () => {
  it("schedules the text for the reply target", async () => {
    const a = chat("a", "10:00");
    home([a, reply("a2", a, "10:05")]);
    scheduling();
    const { user } = renderApp(`/home/${dot.id}`);
    await user.type(await readyBox(), "Ship it");
    const menu = await openMenu(user);
    expect(within(menu).getByText("Send later")).toBeVisible();
    await user.click(within(menu).getByRole("menuitem", { name: /In 1 hour/ }));
    await waitFor(() =>
      expect(backend.callsTo("schedule_message")).toEqual([
        { dotId: dot.id, text: "Ship it", rootRunId: "a", dueAt: AT.toISOString() },
      ]),
    );
    await waitFor(() => expect(box()).toHaveValue(""));
    expect(backend.callsTo("chat")).toEqual([]);
  });

  it("schedules a new topic without a root", async () => {
    home([chat("a", "10:00")]);
    scheduling();
    const { user } = renderApp(`/home/${dot.id}`);
    await readyBox();
    await user.click(screen.getByRole("button", { name: "New topic" }));
    await user.type(box(), "Something else");
    const menu = await openMenu(user);
    await user.click(within(menu).getByRole("menuitem", { name: /In 1 hour/ }));
    await waitFor(() =>
      expect(backend.callsTo("schedule_message")).toEqual([
        { dotId: dot.id, text: "Something else", rootRunId: null, dueAt: AT.toISOString() },
      ]),
    );
  });

  it("sends a picked time as an instant", async () => {
    home([chat("a", "10:00")]);
    scheduling();
    const { user } = renderApp(`/home/${dot.id}`);
    await user.type(await readyBox(), "Ship it");
    const menu = await openMenu(user);
    await user.click(within(menu).getByRole("menuitem", { name: "Pick a date and time…" }));
    const dialog = await screen.findByRole("dialog", { name: "Send later" });
    const picked = new Date(2026, 10, 6, 14, 30);
    const field = within(dialog).getByLabelText("Date and time");
    await user.clear(field);
    await user.type(field, toPicker(picked));
    await user.click(within(dialog).getByRole("button", { name: "Schedule" }));
    await waitFor(() =>
      expect(backend.callsTo("schedule_message")).toEqual([
        { dotId: dot.id, text: "Ship it", rootRunId: "a", dueAt: picked.toISOString() },
      ]),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(box()).toHaveValue("");
  });

  it("shows a refused time in the dialog and keeps the text", async () => {
    home([chat("a", "10:00")]);
    backend.on("schedule_message", () => {
      throw { kind: "invalid", message: "Pick a time at least a minute from now." };
    });
    const { user } = renderApp(`/home/${dot.id}`);
    await user.type(await readyBox(), "Ship it");
    const menu = await openMenu(user);
    await user.click(within(menu).getByRole("menuitem", { name: "Pick a date and time…" }));
    const dialog = await screen.findByRole("dialog", { name: "Send later" });
    await user.click(within(dialog).getByRole("button", { name: "Schedule" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "Pick a time at least a minute from now.",
    );
    expect(box()).toHaveValue("Ship it");
  });

  it("does not offer a preset less than a minute away", async () => {
    home([chat("a", "10:00")]);
    offer.presets = (now) => [
      { id: "evening", label: "This evening (6 PM)", at: new Date(now.getTime() + 30_000) },
      { id: "hour", label: "In 1 hour", at: AT },
    ];
    const { user } = renderApp(`/home/${dot.id}`);
    await user.type(await readyBox(), "Ship it");
    const menu = await openMenu(user);
    expect(within(menu).getByRole("menuitem", { name: /In 1 hour/ })).toBeVisible();
    expect(within(menu).queryByRole("menuitem", { name: /This evening/ })).toBeNull();
  });

  it("refuses in the dialog when the box was locked meanwhile", async () => {
    const onSchedule = vi.fn(() => Promise.resolve());
    const onSend = () => Promise.resolve();
    const user = userEvent.setup();
    const { rerender } = render(
      <Composer disabled={false} hint="" onSend={onSend} onSchedule={onSchedule} />,
    );
    await user.type(screen.getByLabelText("Message"), "Ship it");
    const menu = await openMenu(user);
    await user.click(within(menu).getByRole("menuitem", { name: "Pick a date and time…" }));
    const dialog = await screen.findByRole("dialog", { name: "Send later" });
    // The topic turned busy: the box is locked, as Send is.
    rerender(<Composer disabled hint="" onSend={onSend} onSchedule={onSchedule} />);
    await user.click(within(dialog).getByRole("button", { name: "Schedule" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(SCHEDULE_LOCKED);
    expect(screen.getByRole("dialog", { name: "Send later" })).toBeVisible();
    expect(onSchedule).not.toHaveBeenCalled();
    expect(screen.getByLabelText("Message")).toHaveValue("Ship it");
  });

  it("is locked when Send is", async () => {
    const a = chat("a", "10:00", { status: "running", ended_at: null });
    home([a]);
    renderApp(`/home/${dot.id}`);
    const later = await screen.findByRole("button", { name: "Send later" });
    expect(later).toBeDisabled();
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
  });

  it("needs text, as Send does", async () => {
    home([chat("a", "10:00")]);
    const { user } = renderApp(`/home/${dot.id}`);
    await readyBox();
    expect(screen.getByRole("button", { name: "Send later" })).toBeDisabled();
    await user.type(box(), "Ship it");
    expect(screen.getByRole("button", { name: "Send later" })).toBeEnabled();
  });

  it("is hidden while answering a question", async () => {
    home([chat("a", "10:00")]);
    backend.on("messages_list", () => [{ ...sample.messages[0], run_id: "a" }]);
    const { user } = renderApp(`/home/${dot.id}`);
    const bar = await screen.findByRole("region", { name: "Open question" });
    await readyBox();
    expect(screen.getByRole("button", { name: "Send later" })).toBeInTheDocument();
    await user.click(within(bar).getByRole("button", { name: "Reply to question" }));
    expect(screen.queryByRole("button", { name: "Send later" })).toBeNull();
  });
});
