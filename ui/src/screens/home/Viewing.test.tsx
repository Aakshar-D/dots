import { act, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { backend } from "@/test/backend";
import { chat, dot, home, overview } from "@/test/home";
import { renderApp } from "@/test/render";

/// The `dotId` of every `set_viewing` and every `mark_dot_seen` so far.
const viewing = () => backend.callsTo("set_viewing").map((args) => args.dotId);
const seen = () => backend.callsTo("mark_dot_seen").map((args) => args.dotId);

// The window's focus and the document's visibility, as the test sets them.
let focused = true;
let visibility: DocumentVisibilityState = "visible";
beforeEach(() => {
  focused = true;
  visibility = "visible";
  vi.spyOn(document, "hasFocus").mockImplementation(() => focused);
  vi.spyOn(document, "visibilityState", "get").mockImplementation(() => visibility);
});
afterEach(() => {
  vi.restoreAllMocks();
});

const blur = () =>
  act(() => {
    focused = false;
    window.dispatchEvent(new Event("blur"));
  });
const focus = () =>
  act(() => {
    focused = true;
    window.dispatchEvent(new Event("focus"));
  });
const hide = () =>
  act(() => {
    visibility = "hidden";
    document.dispatchEvent(new Event("visibilitychange"));
  });

describe("the conversation the person is reading", () => {
  it("is viewed and seen while the window has the focus, and not while it is behind", async () => {
    home([chat("a", "10:00")]);
    renderApp(`/home/${dot.id}`);
    await waitFor(() => expect(viewing()).toEqual([dot.id]));
    await waitFor(() => expect(seen()).toEqual([dot.id]));

    blur();
    await waitFor(() => expect(viewing()).toEqual([dot.id, null]));
    focus();
    await waitFor(() => expect(viewing()).toEqual([dot.id, null, dot.id]));
    await waitFor(() => expect(seen()).toEqual([dot.id, dot.id]));
    // Hidden in the tray or minimized.
    hide();
    await waitFor(() => expect(viewing()).toEqual([dot.id, null, dot.id, null]));
    expect(seen()).toEqual([dot.id, dot.id]);
  });

  it("is not viewed when the window is behind as it opens", async () => {
    focused = false;
    home([chat("a", "10:00")]);
    renderApp(`/home/${dot.id}`);
    await waitFor(() => expect(viewing()).toEqual([null]));
    expect(seen()).toEqual([]);
  });

  it("follows a switch to another dot, an older topic, and another screen", async () => {
    const other = { ...dot, id: "dot-other", name: "other" };
    home([chat("a", "10:00"), chat("b", "11:00")]);
    backend.on("dot_overview", () => [overview, { ...overview, dot: other }]);
    const { user, router } = renderApp(`/home/${dot.id}`);
    await waitFor(() => expect(viewing()).toEqual([dot.id]));

    const column = screen.getByRole("navigation", { name: "Dots" });
    await user.click(
      within(within(column).getByRole("listitem", { name: "other" })).getByRole("link"),
    );
    await waitFor(() => expect(viewing()).toEqual([dot.id, null, other.id]));
    await waitFor(() => expect(seen()).toEqual([dot.id, other.id]));

    // An older topic counts as viewing its dot.
    await act(() => router.navigate(`/home/${dot.id}/a`));
    await waitFor(() => expect(viewing()).toEqual([dot.id, null, other.id, null, dot.id]));

    await user.click(screen.getByRole("link", { name: "Inbox" }));
    await waitFor(() => expect(viewing().at(-1)).toBeNull());
    expect(viewing()).toHaveLength(6);
  });

  it("tries again at the next change after a call failed, and says nothing", async () => {
    let refuse = true;
    const answer = () => {
      if (refuse) throw { kind: "internal", message: "the core is busy" };
      return null;
    };
    backend.on("set_viewing", answer);
    backend.on("mark_dot_seen", answer);
    home([chat("a", "10:00")]);
    renderApp(`/home/${dot.id}`);
    await waitFor(() => expect(seen()).toEqual([dot.id]));

    refuse = false;
    blur();
    focus();
    await waitFor(() => expect(seen()).toEqual([dot.id, dot.id]));
    expect(viewing()).toEqual([dot.id, null, dot.id]);
    expect(screen.queryByText("the core is busy")).toBeNull();
  });
});
