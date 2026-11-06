import { FolderOpen } from "lucide-react";

import { commands } from "@/api/commands";
import type { DeployResult as Result, WorkflowRun } from "@/api/types";
import { Button } from "@/components/ui/button";
import { notifyError } from "@/lib/notify";
import { CopyButton } from "@/screens/dot-editor/WebhookPanel";

import { ViewDiffButton } from "./actions";

/// What a deploy did (section 5.5): each branch pushed, with its pull request; or the branches
/// kept, ready, with Open folder and View diff; then the error of a deploy that failed part way.
/// A pull request opens in the browser, as the app's other links to GitHub do; only an https
/// address is a link.
export function DeployResult({ run, result }: { run: WorkflowRun; result: Result }) {
  return (
    <div className="flex flex-col gap-1 text-sm">
      {result.branches.length === 0 ? (
        !result.error && <p className="text-muted-foreground">No branch was deployed.</p>
      ) : (
        <ul aria-label="Deployed branches" className="flex flex-col gap-1">
          {result.branches.map((b) => (
            <li
              key={b.branch}
              aria-label={b.branch}
              className="flex flex-wrap items-center gap-2 rounded-md border bg-background p-2"
            >
              <span className="min-w-0 flex-1 font-mono text-xs break-all">{b.branch}</span>
              <span className="text-xs text-muted-foreground">
                {result.ready ? "Ready" : b.pushed ? "Pushed" : "Not pushed"}
              </span>
              {b.pr_url?.startsWith("https://") && (
                <a href={b.pr_url} target="_blank" rel="noreferrer" className="underline">
                  Pull request
                </a>
              )}
              {b.pr_url && b.auto_merge && (
                <span className="text-xs text-muted-foreground">merges once its checks pass</span>
              )}
              <CopyButton what={`branch ${b.branch}`} text={b.branch} />
              {result.ready && (
                <>
                  <Button
                    variant="outline"
                    size="sm"
                    aria-label={`Open folder of ${b.branch}`}
                    onClick={() => commands.openBranchFolder(run.id, b.branch).catch(notifyError)}
                  >
                    <FolderOpen aria-hidden /> Open folder
                  </Button>
                  <ViewDiffButton runId={run.id} branch={b.branch} commit={b.commit} />
                </>
              )}
            </li>
          ))}
        </ul>
      )}
      {result.error && <p className="text-destructive">{result.error}</p>}
    </div>
  );
}
