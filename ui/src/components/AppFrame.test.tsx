import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { AppFrame } from "@/components/AppFrame";
import { appSrc } from "@/lib/app-bridge";
import { applyTheme } from "@/lib/theme";

/// Loads the frame and returns the port its page was handed, with what arrives on it.
function load(frame: HTMLIFrameElement) {
  const post = vi.spyOn(frame.contentWindow!, "postMessage");
  fireEvent.load(frame);
  expect(post).toHaveBeenCalledTimes(1);
  const [message, origin, transfer] = post.mock.calls[0] as unknown as [
    unknown,
    string,
    MessagePort[],
  ];
  expect(message).toEqual({ type: "dots:connect" });
  // An opaque origin has no name to send to.
  expect(origin).toBe("*");
  const port = transfer[0]!;
  const got: unknown[] = [];
  port.onmessage = (e: MessageEvent) => got.push(e.data);
  return { port, got, post };
}

describe("an app's screen in its frame", () => {
  it("frames the page with scripts only and sends the theme on its first load", async () => {
    render(<AppFrame src={appSrc("expenses", 2)} title="Expenses" tools={[]} call={vi.fn()} />);
    const frame = screen.getByTitle<HTMLIFrameElement>("Expenses");
    expect(frame).toHaveAttribute("sandbox", "allow-scripts");
    expect(frame).toHaveAttribute("src", "http://dots-app.localhost/app/expenses/2/index.html");
    const { got } = load(frame);
    await waitFor(() => expect(got).toHaveLength(1));
    expect(got[0]).toMatchObject({ type: "theme", mode: "light" });
    expect((got[0] as { tokens: Record<string, string> }).tokens.primary).toBe("#0c7765");
  });

  it("calls a listed tool as the page asks and refuses one it does not list", async () => {
    const call = vi.fn(async (tool: string) => (tool === "list_expenses" ? [{ item: "Ink" }] : 0));
    render(
      <AppFrame src={appSrc("expenses", 1)} title="Expenses" tools={["list_expenses"]} call={call} />,
    );
    const { port, got } = load(screen.getByTitle<HTMLIFrameElement>("Expenses"));
    port.postMessage({ type: "call", id: 1, tool: "list_expenses", input: { month: "2026-10" } });
    port.postMessage({ type: "call", id: 2, tool: "drop_all", input: {} });
    port.postMessage({ type: "call", id: "3", tool: "list_expenses" });
    port.postMessage({ type: "call", id: 4, tool: "list_expenses", input: {}, actor: "dot" });
    await waitFor(() => expect(got).toHaveLength(3));
    expect(call).toHaveBeenCalledTimes(1);
    expect(call).toHaveBeenCalledWith("list_expenses", { month: "2026-10" });
    expect(got).toContainEqual({ type: "result", id: 1, ok: true, result: [{ item: "Ink" }] });
    expect(got).toContainEqual({
      type: "result",
      id: 2,
      ok: false,
      error: "this app has no tool drop_all",
    });
  });

  it("answers a failed call with its error", async () => {
    const call = vi.fn(async () => {
      throw new Error("amount: must be at least 0");
    });
    render(<AppFrame src={appSrc("expenses", 1)} title="Expenses" tools={["add"]} call={call} />);
    const { port, got } = load(screen.getByTitle<HTMLIFrameElement>("Expenses"));
    port.postMessage({ type: "call", id: 7, tool: "add", input: { amount: -1 } });
    await waitFor(() =>
      expect(got).toContainEqual({
        type: "result",
        id: 7,
        ok: false,
        error: "amount: must be at least 0",
      }),
    );
  });

  it("opens a run the page asks for, and refuses where the screen cannot open runs", async () => {
    const openRun = vi.fn(async () => {});
    const { unmount } = render(
      <AppFrame
        src={appSrc("projects", 1)}
        title="Projects"
        tools={[]}
        call={vi.fn()}
        openRun={openRun}
      />,
    );
    const { port, got } = load(screen.getByTitle<HTMLIFrameElement>("Projects"));
    port.postMessage({ type: "open_run", id: 1, run_id: "r1" });
    await waitFor(() =>
      expect(got).toContainEqual({ type: "result", id: 1, ok: true, result: null }),
    );
    expect(openRun).toHaveBeenCalledWith("r1");
    unmount();
    render(<AppFrame src={appSrc("projects", 1)} title="Preview" tools={[]} call={vi.fn()} />);
    const preview = load(screen.getByTitle<HTMLIFrameElement>("Preview"));
    preview.port.postMessage({ type: "open_run", id: 2, run_id: "r1" });
    await waitFor(() =>
      expect(preview.got).toContainEqual({
        type: "result",
        id: 2,
        ok: false,
        error: "this screen cannot open runs",
      }),
    );
  });

  it("sends the theme again when it changes after the page loaded", async () => {
    render(<AppFrame src={appSrc("expenses", 1)} title="Expenses" tools={[]} call={vi.fn()} />);
    const { got } = load(screen.getByTitle<HTMLIFrameElement>("Expenses"));
    await waitFor(() => expect(got).toHaveLength(1));
    act(() => {
      applyTheme("dark");
    });
    try {
      await waitFor(() => expect(got.at(-1)).toMatchObject({ type: "theme", mode: "dark" }));
    } finally {
      act(() => {
        applyTheme("light");
      });
    }
  });

  it("tells the page of each change to the app's data", async () => {
    const { rerender } = render(
      <AppFrame src={appSrc("expenses", 1)} title="Expenses" tools={[]} call={vi.fn()} changes={0} />,
    );
    const { got } = load(screen.getByTitle<HTMLIFrameElement>("Expenses"));
    rerender(
      <AppFrame src={appSrc("expenses", 1)} title="Expenses" tools={[]} call={vi.fn()} changes={1} />,
    );
    await waitFor(() => expect(got).toContainEqual({ type: "changed" }));
  });

  it("cuts off a page that navigated away, and loads it afresh when asked", async () => {
    const user = userEvent.setup();
    render(<AppFrame src={appSrc("expenses", 1)} title="Expenses" tools={[]} call={vi.fn()} />);
    const first = screen.getByTitle<HTMLIFrameElement>("Expenses");
    const { post } = load(first);
    act(() => {
      fireEvent.load(first);
    });
    // The navigated page got no port: the one message was the first load's.
    expect(post).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("alert")).toHaveTextContent(
      "The app's screen left its page, so Dots stopped answering it.",
    );
    expect(screen.queryByTitle("Expenses")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Load the app again" }));
    const again = screen.getByTitle<HTMLIFrameElement>("Expenses");
    load(again);
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
