import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { commands } from "@/api/commands";
import type { UsageCountsState } from "@/api/types";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { errorMessage } from "@/lib/notify";
import { keys } from "@/lib/query";

/// What is wrong with a destination address, mirroring `usage_counts::check_url` in the core:
/// `https://` only, except `http://localhost` and `http://127.0.0.1` for testing on this PC.
export function usageUrlError(text: string): string | undefined {
  const url = text.trim();
  if (url === "") return "Enter the address to send the counts to.";
  if (url.length > 2048) return "That address is too long.";
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return "That is not a web address. It starts with https://";
  }
  const scheme = parsed.protocol.replace(/:$/, "");
  if (scheme !== "https" && scheme !== "http") {
    return "The address must start with https:// (http:// is allowed only for localhost or 127.0.0.1).";
  }
  if (parsed.hostname === "") return "That address has no server name.";
  if (parsed.username !== "" || parsed.password !== "") {
    return "The address must not hold a user name or password.";
  }
  if (scheme === "http" && parsed.hostname !== "localhost" && parsed.hostname !== "127.0.0.1") {
    return "The address must start with https:// (http:// is allowed only for localhost or 127.0.0.1).";
  }
  return undefined;
}

const WHAT_IS_SENT =
  "Only counts: how many runs started, finished or failed, messages sent and approvals asked each hour, and how many dots you have. Never names, files, messages or anything a dot wrote.";

/// Settings › Share usage counts (usage counts spec): off until turned on with a destination.
/// It saves at once, apart from the rest of Settings.
export function UsageCountsSection() {
  const queryClient = useQueryClient();
  const state = useQuery({ queryKey: keys.usageCounts, queryFn: commands.usageCountsState });
  const [url, setUrl] = useState<string | null>(null);
  const [showing, setShowing] = useState(false);
  const stored = state.data;
  // The field shows the stored address until the person types.
  const text = url ?? stored?.url ?? "";

  const keep = (next: UsageCountsState) => {
    queryClient.setQueryData(keys.usageCounts, next);
    void queryClient.invalidateQueries({ queryKey: keys.usageCountsPreview });
  };
  const set = useMutation({
    mutationFn: (v: { enabled: boolean; url: string | null }) =>
      commands.setUsageCounts(v.enabled, v.url),
    onSuccess: keep,
  });
  const reset = useMutation({ mutationFn: commands.resetUsageCountsId, onSuccess: keep });
  const preview = useQuery({
    queryKey: keys.usageCountsPreview,
    queryFn: commands.usageCountsPreview,
    enabled: showing,
  });

  const enabled = stored?.enabled ?? false;
  const urlError = text.trim() === "" ? undefined : usageUrlError(text);
  const urlOk = usageUrlError(text) === undefined;
  const changedUrl = text.trim() !== (stored?.url ?? "") && urlOk;

  return (
    <fieldset className="flex max-w-3xl flex-col gap-3 rounded-lg border p-4">
      <legend className="px-1 text-sm font-semibold">Share usage counts</legend>
      <p className="text-xs text-muted-foreground">
        Dots can send a short report once an hour to an address you choose. It is off unless you
        turn it on. {WHAT_IS_SENT}
      </p>
      <Field
        label="Send to"
        htmlFor="usage-counts-url"
        error={urlError}
        hint="An https:// address. http:// works only for localhost or 127.0.0.1, for testing."
      >
        <div className="flex gap-2">
          <Input
            id="usage-counts-url"
            className="font-mono"
            placeholder="https://"
            value={text}
            onChange={(e) => setUrl(e.target.value)}
          />
          {enabled && changedUrl && (
            <Button
              type="button"
              variant="outline"
              disabled={set.isPending}
              onClick={() => set.mutate({ enabled: true, url: text.trim() })}
            >
              Save address
            </Button>
          )}
        </div>
      </Field>
      <div className="flex items-center gap-2">
        <Switch
          id="usage-counts-enabled"
          checked={enabled}
          disabled={!stored || set.isPending || (!enabled && !urlOk)}
          onCheckedChange={(on) =>
            set.mutate(on ? { enabled: true, url: text.trim() } : { enabled: false, url: null })
          }
        />
        <Label htmlFor="usage-counts-enabled">Send usage counts every hour</Label>
      </div>
      {set.isError && <p className="text-sm text-destructive">{errorMessage(set.error)}</p>}
      {state.isError && <p className="text-sm text-destructive">{errorMessage(state.error)}</p>}

      <dl className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1 text-xs">
        <dt className="text-muted-foreground">Last sent</dt>
        <dd>
          {stored?.last_sent_at ? new Date(stored.last_sent_at).toLocaleString() : "Not sent yet"}
        </dd>
        <dt className="text-muted-foreground">Random ID</dt>
        <dd className="flex items-center gap-2">
          <span className="font-mono">
            {stored?.install_id ?? "Made when you first turn this on"}
          </span>
          {stored?.install_id && (
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={reset.isPending}
              onClick={() => reset.mutate()}
            >
              Reset ID
            </Button>
          )}
        </dd>
      </dl>
      {stored?.last_error && (
        <p role="alert" className="text-sm text-destructive">
          Last send failed: {stored.last_error}
        </p>
      )}
      {reset.isError && <p className="text-sm text-destructive">{errorMessage(reset.error)}</p>}

      <div>
        <Button type="button" size="sm" variant="outline" onClick={() => setShowing((s) => !s)}>
          {showing ? "Hide what gets sent" : "Show what gets sent"}
        </Button>
      </div>
      {showing && (
        <>
          <p className="text-xs text-muted-foreground">
            The next report, exactly as it would be sent. The ID is random and can be reset.
          </p>
          {preview.isError ? (
            <p className="text-sm text-destructive">{errorMessage(preview.error)}</p>
          ) : (
            <pre
              aria-label="The next report"
              className="overflow-x-auto rounded-md border bg-muted p-2 font-mono text-xs"
            >
              {preview.data ? JSON.stringify(preview.data, null, 2) : "Loading…"}
            </pre>
          )}
        </>
      )}
    </fieldset>
  );
}
