import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useNavigate } from "react-router";

import { commands } from "@/api/commands";
import type { Project, Workflow } from "@/api/types";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { Textarea } from "@/components/ui/textarea";
import { errorMessage } from "@/lib/notify";
import { invalidate } from "@/lib/query";

/// "Run now", or "Test run" with `test` (teams spec section 5.1): the task and the project, then
/// the run starts on the saved workflow and its page opens.
export function RunNowDialog(props: {
  workflow: Workflow;
  projects: Project[];
  test?: boolean;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const [task, setTask] = useState("");
  const [projectId, setProjectId] = useState(props.projects[0]?.id ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const test = props.test ?? false;

  async function start() {
    setBusy(true);
    setError(undefined);
    try {
      const run = await commands.workflowRunStart(props.workflow.id, projectId, task.trim(), test);
      invalidate(qc, ["workflow-runs"]);
      navigate(`/workflow-runs/${run.id}`);
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  }

  const missing =
    task.trim() === ""
      ? "Say what the task is first."
      : projectId === ""
        ? "Add a project to the team first."
        : undefined;
  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>
            {test ? "Test run" : "Run"} {props.workflow.name}
          </DialogTitle>
          <DialogDescription>
            {test
              ? "A real run of the saved workflow, except that its deploys push nothing: the branches are kept for you to look at."
              : "The saved workflow runs on the project, step by step, from the task."}
          </DialogDescription>
        </DialogHeader>
        <Field label="Task" htmlFor="run-task">
          <Textarea
            id="run-task"
            rows={5}
            placeholder="What should the team do?"
            value={task}
            onChange={(e) => setTask(e.target.value)}
          />
        </Field>
        <Field label="Project" htmlFor="run-project">
          <NativeSelect
            id="run-project"
            value={projectId}
            onChange={(e) => setProjectId(e.target.value)}
          >
            {props.projects.map((project) => (
              <NativeSelectOption key={project.id} value={project.id}>
                {project.name}
              </NativeSelectOption>
            ))}
          </NativeSelect>
        </Field>
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
        <DialogFooter>
          <Button type="button" variant="outline" onClick={props.onClose}>
            Cancel
          </Button>
          <Button
            type="button"
            disabled={busy || missing !== undefined}
            title={missing}
            onClick={() => void start()}
          >
            {test ? "Start test run" : "Start run"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
