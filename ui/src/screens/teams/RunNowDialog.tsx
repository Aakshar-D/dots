import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useNavigate } from "react-router";

import { commands } from "@/api/commands";
import type { Project, Workflow } from "@/api/types";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
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
  // "": the team's own folder.
  const [projectId, setProjectId] = useState(props.projects[0]?.id ?? "");
  const fields = props.workflow.definition.inputs;
  const [inputs, setInputs] = useState<Record<string, string>>(() =>
    Object.fromEntries(fields.map((f) => [f.name, f.default ?? ""])),
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const test = props.test ?? false;

  async function start() {
    setBusy(true);
    setError(undefined);
    try {
      const given = Object.fromEntries(
        Object.entries(inputs).filter(([, text]) => text.trim() !== ""),
      );
      const run = await commands.workflowRunStart(
        props.workflow.id,
        projectId === "" ? null : projectId,
        task.trim(),
        test,
        given,
      );
      invalidate(qc, ["workflow-runs"]);
      navigate(`/workflow-runs/${run.id}`);
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  }

  const needed = fields.find((f) => f.required && (inputs[f.name] ?? "").trim() === "");
  const missing =
    task.trim() === ""
      ? "Say what the task is first."
      : needed
        ? `Give "${needed.label.trim() || needed.name}" first.`
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
              ? "A real run of the saved workflow, except that nothing leaves this PC: its deploys push nothing and its actions send nothing."
              : "The saved workflow runs step by step, from the task."}
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
        <Field
          label="Project"
          htmlFor="run-project"
          hint="A project of the team, or the folder Sage keeps for the team."
        >
          <NativeSelect
            id="run-project"
            value={projectId}
            onChange={(e) => setProjectId(e.target.value)}
          >
            <NativeSelectOption value="">The team's own folder</NativeSelectOption>
            {props.projects.map((project) => (
              <NativeSelectOption key={project.id} value={project.id}>
                {project.name}
              </NativeSelectOption>
            ))}
          </NativeSelect>
        </Field>
        {fields.map((field) => {
          const id = `run-input-${field.name}`;
          const value = inputs[field.name] ?? "";
          const change = (text: string) => setInputs((all) => ({ ...all, [field.name]: text }));
          return (
            <Field
              key={field.name}
              label={`${field.label.trim() || field.name}${field.required ? "" : " (optional)"}`}
              htmlFor={id}
            >
              {field.kind === "long_text" ? (
                <Textarea id={id} rows={3} value={value} onChange={(e) => change(e.target.value)} />
              ) : (
                <Input
                  id={id}
                  type={field.kind === "date_time" ? "datetime-local" : "text"}
                  value={value}
                  onChange={(e) => change(e.target.value)}
                />
              )}
            </Field>
          );
        })}
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
