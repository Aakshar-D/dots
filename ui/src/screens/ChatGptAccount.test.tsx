import { QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { createQueryClient } from "@/lib/query";
import { ChatGptAccount } from "@/screens/ChatGptAccount";
import { backend, later } from "@/test/backend";
import { renderApp } from "@/test/render";

const signedOut = { configured: true, signing_in: false, account: null, setup_error: null, expires_at: null };
const signedIn = { ...signedOut, expires_at: new Date(Date.now() + 8 * 60 * 60 * 1000).toISOString(), account: { name: "Alex", email: "alex@example.com" } };

describe("ChatGPT account in Settings", () => {
  it("explains missing registration and does not start an unavailable login", async () => {
    backend.on("chatgpt_account", () => ({
      ...signedOut, configured: false,
      setup_error: "ChatGPT sign-in needs a registered OpenAI client for Dots.",
    }));
    renderApp("/settings");
    expect(await screen.findByRole("button", { name: "Sign in with ChatGPT" })).toBeDisabled();
    expect(await screen.findByText(/needs a registered OpenAI client/)).toBeVisible();
    expect(backend.callsTo("chatgpt_sign_in")).toEqual([]);
  });

  it("waits for browser sign-in and then shows the verified account", async () => {
    backend.on("chatgpt_account", () => signedOut);
    const login = later<typeof signedIn>();
    backend.on("chatgpt_sign_in", () => login.answer.then((result) => {
      backend.on("chatgpt_account", () => result);
      return result;
    }));
    const { user } = renderApp("/settings");
    await user.click(await screen.findByRole("button", { name: "Sign in with ChatGPT" }));
    expect(await screen.findByText("Complete sign-in in your browser.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Signing in…"})).toBeDisabled();
    await act(async () => login.give(signedIn));
    expect(await screen.findByText("alex@example.com")).toBeVisible();
    expect(screen.getByText("Alex")).toBeVisible();
    expect(screen.getByRole("button", { name: "Sign out" })).toBeEnabled();
    expect(backend.callsTo("update_settings")).toEqual([]);
  });

  it("shows a failed sign-in and allows another attempt", async () => {
    backend.on("chatgpt_account", () => signedOut);
    backend.on("chatgpt_sign_in", () => {
      throw { kind: "invalid", message: "Sign-in could not be verified. Try again." };
    });
    const { user } = renderApp("/settings");
    await user.click(await screen.findByRole("button", { name: "Sign in with ChatGPT" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Sign-in could not be verified");
    expect(screen.getByRole("button", { name: "Sign in with ChatGPT" })).toBeEnabled();
  });

  it("signs out and clears the account without changing settings", async () => {
    backend.on("chatgpt_account", () => signedIn);
    backend.on("chatgpt_sign_out", () => {
      backend.on("chatgpt_account", () => signedOut);
      return signedOut;
    });
    const { user } = renderApp("/settings");
    await user.click(await screen.findByRole("button", { name: "Sign out" }));
    await waitFor(() => expect(screen.queryByText("alex@example.com")).toBeNull());
    expect(screen.getByRole("button", { name: "Sign in with ChatGPT" })).toBeEnabled();
    expect(backend.callsTo("chatgpt_sign_out")).toEqual([{}]);
    expect(backend.callsTo("update_settings")).toEqual([]);
  });

  it("cancels pending login without showing cancellation as an error", async () => {
    backend.on("chatgpt_account", () => signedOut);
    const login = later<typeof signedIn>();
    backend.on("chatgpt_sign_in", () => login.answer.then((result) => {
      backend.on("chatgpt_account", () => result);
      return result;
    }));
    backend.on("chatgpt_sign_out", () => {
      login.fail({ kind: "cancelled", message: "Sign-in cancelled." });
      return signedOut;
    });
    const { user } = renderApp("/settings");
    await user.click(await screen.findByRole("button", { name: "Sign in with ChatGPT" }));
    await user.click(await screen.findByRole("button", { name: "Cancel sign-in" }));
    expect(await screen.findByRole("button", { name: "Sign in with ChatGPT" })).toBeEnabled();
    expect(screen.queryByRole("alert")).toBeNull();
  });
  it("returns to sign-in when the session expires while Settings stays focused", async () => {
    vi.useFakeTimers();
    try {
      backend.on("chatgpt_account", () => ({
        ...signedIn, expires_at: new Date(Date.now() + 1000).toISOString(),
      }));
      render(
        <QueryClientProvider client={createQueryClient()}>
          <ChatGptAccount />
        </QueryClientProvider>,
      );
      await act(async () => { await vi.advanceTimersByTimeAsync(100); });
      expect(screen.getByText("Signed in with ChatGPT")).toBeVisible();
      backend.on("chatgpt_account", () => signedOut);
      await act(async () => { await vi.advanceTimersByTimeAsync(1100); });
      expect(screen.queryByText("Signed in with ChatGPT")).toBeNull();
      expect(screen.getByRole("button", { name: "Sign in with ChatGPT" })).toBeEnabled();
    } finally {
      vi.useRealTimers();
    }
  });

  it("rechecks expiry if the native monotonic session clock is slightly behind", async () => {
    vi.useFakeTimers();
    try {
      const expiresAt = new Date(Date.now() + 1000).toISOString();
      backend.on("chatgpt_account", () =>
        backend.callsTo("chatgpt_account").length < 3
          ? { ...signedIn, expires_at: expiresAt }
          : signedOut,
      );
      render(
        <QueryClientProvider client={createQueryClient()}>
          <ChatGptAccount />
        </QueryClientProvider>,
      );
      await act(async () => { await vi.advanceTimersByTimeAsync(100); });
      expect(screen.getByText("Signed in with ChatGPT")).toBeVisible();
      await act(async () => { await vi.advanceTimersByTimeAsync(1100); });
      expect(screen.getByText("Signed in with ChatGPT")).toBeVisible();
      await act(async () => { await vi.advanceTimersByTimeAsync(1100); });
      expect(screen.queryByText("Signed in with ChatGPT")).toBeNull();
      expect(backend.callsTo("chatgpt_account")).toHaveLength(3);
    } finally {
      vi.useRealTimers();
    }
  });

});
