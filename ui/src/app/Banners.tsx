import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { Status } from "@/api/types";
import { Button } from "@/components/ui/button";
import { notifyError } from "@/lib/notify";
import { keys } from "@/lib/query";

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

/// What the user must know on every screen: no local server, no or an untested CLI, settings
/// waiting for a restart, paused schedules.
export function Banners() {
  const qc = useQueryClient();
  const status = useQuery({ queryKey: keys.status, queryFn: commands.status }).data;
  const take = (next: Status) => qc.setQueryData(keys.status, next);
  const markTested = useMutation({
    mutationFn: commands.markClaudeTested,
    onSuccess: take,
    onError: notifyError,
  });
  const resume = useMutation({
    mutationFn: () => commands.setSchedulesPaused(false),
    onSuccess: (next) => {
      take(next);
      void qc.invalidateQueries({ queryKey: keys.overview });
    },
    onError: notifyError,
  });
  const restart = useMutation({ mutationFn: commands.restartApp, onError: notifyError });
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
          <Button
            size="sm"
            variant="outline"
            disabled={restart.isPending}
            onClick={() => restart.mutate()}
          >
            Restart Dots
          </Button>
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
