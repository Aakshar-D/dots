import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { commands } from "@/api/commands";
import type { CheckOutcome, UpdateStatus } from "@/api/types";
import { Link } from "react-router";

import { ConfirmButton } from "@/components/ConfirmButton";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { WhatsNewDialog } from "@/components/WhatsNewDialog";
import { localTime } from "@/lib/format";
import { errorMessage } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { VaultChooser } from "@/screens/dot-editor/WatchersTab";

/// "Dots 0.3.0 · up to date · checked 10:42", "· 0.3.1 is ready", "· never checked".
function statusLine(s: UpdateStatus): string {
  if (s.ready) return `Dots ${s.version} · ${s.ready} is ready`;
  if (s.checked_at) return `Dots ${s.version} · up to date · checked ${localTime(s.checked_at)}`;
  return `Dots ${s.version} · never checked`;
}

function outcomeLine(o: CheckOutcome): string | null {
  switch (o.state) {
    case "up_to_date":
      return "Up to date";
    case "ready":
      return `${o.version} is ready`;
    case "failed":
      return o.error;
    case "off":
      return null;
  }
}

/// Settings › Updates (updates UI spec section 4.1): the version, Check for updates, What's
/// new, the idle-install switch (saved with the other settings), Repair and Go back.
export function UpdatesSection(props: {
  autoInstall: boolean;
  onAutoInstall: (on: boolean) => void;
  publishToken: string | null;
  onPublishToken: (name: string | null) => void;
}) {
  const [choosing, setChoosing] = useState(false);
  const publisher = useQuery({ queryKey: keys.publisher, queryFn: commands.publisherStatus }).data;
  const qc = useQueryClient();
  const status = useQuery({ queryKey: keys.updateStatus, queryFn: commands.updateStatus }).data;
  const [notesOf, setNotesOf] = useState<string | null>(null);
  const settled = () => invalidate(qc, keys.updateStatus);
  const check = useMutation({ mutationFn: commands.updateCheckNow, onSettled: settled });
  const repair = useMutation({ mutationFn: commands.updateRepair, onSettled: settled });
  const goBack = useMutation({ mutationFn: commands.updateGoBack, onSettled: settled });
  const failed = repair.error ?? goBack.error;
  return (
    <fieldset className="flex max-w-3xl flex-col gap-3 rounded-lg border p-4">
      <legend className="px-1 text-sm font-semibold">Updates</legend>
      {status && !status.checks_on && (
        <p className="text-sm text-muted-foreground">Updates are off in this copy</p>
      )}
      {status?.checks_on && (
        <>
          <p className="text-sm">{statusLine(status)}</p>
          <div className="flex flex-wrap items-center gap-2">
            <Button
              size="sm"
              variant="outline"
              disabled={check.isPending}
              onClick={() => check.mutate()}
            >
              Check for updates
            </Button>
            <Button size="sm" variant="outline" onClick={() => setNotesOf(status.version)}>
              What's new
            </Button>
            {check.isPending && <span className="text-sm text-muted-foreground">Checking…</span>}
            {!check.isPending && check.data && outcomeLine(check.data) && (
              <span
                role={check.data.state === "failed" ? "alert" : "status"}
                className={
                  check.data.state === "failed" ? "text-sm text-destructive" : "text-sm"
                }
              >
                {outcomeLine(check.data)}
              </span>
            )}
          </div>
          <div className="flex items-center gap-2">
            <Switch
              id="settings-auto-install"
              checked={props.autoInstall}
              onCheckedChange={props.onAutoInstall}
            />
            <Label htmlFor="settings-auto-install">Install updates by itself when idle</Label>
          </div>
          <p className="text-xs text-muted-foreground">
            Off: a strip appears and you choose when to restart.
          </p>
          <div className="flex flex-wrap gap-2">
            <ConfirmButton
              title="Repair Dots"
              description={`Dots will download version ${status.version} again, back up your data and reinstall it. Runs that are working will be stopped.`}
              confirmLabel="Repair"
              disabled={repair.isPending}
              onConfirm={() => repair.mutate()}
            >
              Repair Dots…
            </ConfirmButton>
            {status.go_back && (
              <ConfirmButton
                title="Go back to an older version"
                description={`Go back to ${status.go_back}? Your data from before the update comes back with it; anything done since the update is lost.`}
                confirmLabel="Go back"
                disabled={goBack.isPending}
                onConfirm={() => goBack.mutate()}
              >
                Go back to {status.go_back}…
              </ConfirmButton>
            )}
          </div>
          {failed && (
            <p role="alert" className="text-sm text-destructive">
              {errorMessage(failed)}
            </p>
          )}
        </>
      )}
      {status?.publisher ? (
        <Link to="/releases" className="text-sm underline">
          Publish versions of Dots…
        </Link>
      ) : (
        <div className="flex flex-col items-start gap-2">
          <Button size="sm" variant="ghost" onClick={() => setChoosing((c) => !c)}>
            Publish versions of Dots (for its developers)…
          </Button>
          {choosing && (
            <div className="flex flex-col gap-1">
              <Label htmlFor="settings-publish-token">GitHub token</Label>
              <VaultChooser
                id="settings-publish-token"
                value={props.publishToken ?? ""}
                onChange={(name) => props.onPublishToken(name || null)}
                initialName="GITHUB_PUBLISH"
              />
              <p className="text-xs text-muted-foreground">
                A classic token with the repo and workflow scopes.
              </p>
              {publisher?.state.state === "refused" && (
                <p role="alert" className="text-sm text-destructive">
                  {publisher.state.reason}
                </p>
              )}
            </div>
          )}
        </div>
      )}
      {notesOf && (
        <WhatsNewDialog
          version={notesOf}
          open
          onOpenChange={(open) => !open && setNotesOf(null)}
        />
      )}
    </fieldset>
  );
}
