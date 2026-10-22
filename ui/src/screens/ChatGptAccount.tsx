import { useEffect } from "react";
import { useIsMutating, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { CommandError, commands } from "@/api/commands";
import type { ChatGptStatus } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import { invalidate, keys } from "@/lib/query";

const signInKey = ["chatgpt-sign-in"] as const;
const signOutKey = ["chatgpt-sign-out"] as const;

export function ChatGptAccount() {
  const qc = useQueryClient();
  const account = useQuery({
    queryKey: keys.chatGptAccount,
    queryFn: commands.chatGptAccount,
    staleTime: 0,
    refetchOnWindowFocus: true,
  });
  const take = (status: ChatGptStatus) => qc.setQueryData(keys.chatGptAccount, status);
  const refresh = () => invalidate(qc, keys.chatGptAccount);
  const login = useMutation({
    mutationKey: signInKey,
    mutationFn: commands.chatGptSignIn,
    onSuccess: take,
    onSettled: refresh,
  });
  const logout = useMutation({
    mutationKey: signOutKey,
    mutationFn: commands.chatGptSignOut,
    onSuccess: take,
    onSettled: refresh,
  });
  const signingIn = useIsMutating({ mutationKey: signInKey }) > 0 || !!account.data?.signing_in;
  const signingOut = useIsMutating({ mutationKey: signOutKey }) > 0;
  const cancelled = login.error instanceof CommandError && login.error.kind === "cancelled";
  const identity = account.data?.account;
  const expiresAt = account.data?.expires_at;
  useEffect(() => {
    if (!expiresAt) return;
    const delay = Math.max(1000, Date.parse(expiresAt) - Date.now());
    const timer = window.setTimeout(() => invalidate(qc, keys.chatGptAccount), delay);
    return () => window.clearTimeout(timer);
  }, [expiresAt, qc, account.dataUpdatedAt]);

  return (
    <fieldset aria-label="ChatGPT account" className="flex max-w-3xl flex-col gap-3 rounded-lg border p-4">
      <legend className="px-1 text-sm font-semibold">Account</legend>
      {account.isError && (
        <>
          <ErrorNote error={account.error} />
          <Button type="button" size="sm" variant="outline" className="self-start" onClick={() => void account.refetch()}>
            Retry account
          </Button>
        </>
      )}
      {login.isError && !cancelled && <ErrorNote error={login.error} />}
      {logout.isError && <ErrorNote error={logout.error} />}
      {identity ? (
        <>
          <div role="status" className="flex flex-col gap-1">
            <p className="font-medium">Signed in with ChatGPT</p>
            {identity.name && <p>{identity.name}</p>}
            {identity.email && <p className="break-all text-muted-foreground">{identity.email}</p>}
          </div>
          <Button type="button" size="sm" variant="outline" className="self-start"
            disabled={signingOut} onClick={() => logout.mutate()}>
            {signingOut ? "Signing out…" : "Sign out"}
          </Button>
        </>
      ) : (
        <>
          <p className="text-muted-foreground">
            Sign into Dots with your ChatGPT account.
          </p>
          <div className="flex flex-wrap items-center gap-3">
            <Button type="button" disabled={!account.data?.configured || signingIn || signingOut}
              onClick={() => { logout.reset(); login.mutate(); }}>
              {signingIn ? "Signing in…" : "Sign in with ChatGPT"}
            </Button>
            {signingIn && (
              <Button type="button" variant="outline" disabled={signingOut}
                onClick={() => { login.reset(); logout.mutate(); }}>
                Cancel sign-in
              </Button>
            )}
          </div>
          {signingIn && <p role="status">Complete sign-in in your browser.</p>}
          {account.data?.setup_error && <p className="text-muted-foreground">{account.data.setup_error}</p>}
          {account.isPending && <p role="status">Checking account…</p>}
        </>
      )}
      <p className="text-xs text-muted-foreground">
        Sign-in lasts until you quit Dots or for eight hours. Account changes apply immediately.
      </p>
    </fieldset>
  );
}
