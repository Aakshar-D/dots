import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { Status } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { Button } from "@/components/ui/button";
import { notifyError } from "@/lib/notify";
import { invalidate, keys, takeStatus } from "@/lib/query";

function Banner(props: { tone: "error" | "warning" | "info"; children: ReactNode }) {
  const tone = {
    error: "border-red-500/40 bg-red-500/15",
    warning: "border-amber-500/40 bg-amber-500/15",
    info: "border-border bg-muted",
  }[props.tone];
  return (
    <div
      role={props.tone === "error" ? "alert" : "status"}
      className={`flex items-center gap-3 border-b px-5 py-2 ${tone}`}
    >
      {props.children}
    </div>
  );
}

/// "Restart Dots". With no run that has not ended it restarts at once. Otherwise it asks
/// first: `active` names the dots that have such a run, and is `undefined` when that is not
/// known, which asks too.
function RestartButton(props: {
  active: string[] | undefined;
  disabled: boolean;
  onRestart: () => void;
}) {
  const { active } = props;
  if (active?.length === 0) {
    return (
      <Button size="sm" variant="outline" disabled={props.disabled} onClick={props.onRestart}>
        Restart Dots
      </Button>
    );
  }
  const found = !active
    ? "Dots could not check whether a run is active."
    : active.length === 1
      ? `${active[0]} has a run that has not ended.`
      : `${active.join(", ")} have runs that have not ended.`;
  return (
    <ConfirmButton
      title="Restart Dots now?"
      description={
        <>
          {found} A run that is working is stopped by the restart and ends as failed
          ("interrupted"). Runs that wait in the queue or for an approval continue after it.
        </>
      }
      confirmLabel="Restart now"
      onConfirm={props.onRestart}
      disabled={props.disabled}
    >
      Restart Dots
    </ConfirmButton>
  );
}

/// What the user must know on every screen: no local server, no or an untested CLI, settings
/// waiting for a restart, paused schedules.
export function Banners() {
  const qc = useQueryClient();
  const status = useQuery({ queryKey: keys.status, queryFn: commands.status }).data;
  const take = (next: Status) => takeStatus(qc, next);
  const markTested = useMutation({
    mutationFn: commands.markClaudeTested,
    onSuccess: take,
    onError: notifyError,
  });
  const resume = useMutation({
    mutationFn: () => commands.setSchedulesPaused(false),
    onSuccess: (next) => {
      take(next);
      invalidate(qc, keys.overview);
    },
    onError: notifyError,
  });
  const restart = useMutation({ mutationFn: commands.restartApp, onError: notifyError });
  // A restart ends a run that is working as failed ("interrupted"), so the restart button asks
  // first while a run has not ended. The Dots list holds that: each dot's newest run that is
  // queued, running or awaiting approval. It is asked for only while a restart is on offer,
  // and runtime events keep it current like on the Dots screen.
  const overview = useQuery({
    queryKey: keys.overview,
    queryFn: commands.dotOverview,
    enabled: status?.restart_required === true,
  });
  if (!status) return null;
  const { claude } = status;
  const settings = (
    <Link to="/settings" className="underline">
      Open Settings
    </Link>
  );
  return (
    <>
      {status.server_error && (
        <Banner tone="error">
          <span className="flex-1">
            The local server is not running: {status.server_error}. Webhooks are off and Claude
            dots cannot run.
          </span>
          {settings}
        </Banner>
      )}
      {!claude.program && (
        <Banner tone="error">
          <span className="flex-1">
            The claude CLI was not found. Claude dots cannot run until it is installed or its
            path is set.
          </span>
          {settings}
        </Banner>
      )}
      {claude.mismatch && (
        <Banner tone="warning">
          <span className="flex-1">
            Claude CLI {claude.version} has not been tested with Dots (tested:{" "}
            {claude.tested_version}).
          </span>
          <Button
            size="sm"
            variant="outline"
            disabled={markTested.isPending}
            onClick={() => markTested.mutate()}
          >
            Mark as tested
          </Button>
        </Banner>
      )}
      {status.restart_required && (
        <Banner tone="warning">
          <span className="flex-1">Some settings take effect after a restart.</span>
          <RestartButton
            // `undefined` until the list is here, and when it could not be loaded.
            active={overview.data?.filter((row) => row.active_run).map((row) => row.dot.name)}
            disabled={restart.isPending || overview.isPending}
            onRestart={() => restart.mutate()}
          />
        </Banner>
      )}
      {status.schedules_paused && (
        <Banner tone="info">
          <span className="flex-1">Schedules are paused. No dot runs on its schedule.</span>
          <Button
            size="sm"
            variant="outline"
            disabled={resume.isPending}
            onClick={() => resume.mutate()}
          >
            Resume schedules
          </Button>
        </Banner>
      )}
    </>
  );
}
