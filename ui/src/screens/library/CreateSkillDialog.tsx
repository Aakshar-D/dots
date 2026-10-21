import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";

import { commands } from "@/api/commands";
import type { DraftRequest, DraftSummary } from "@/api/types";
import { Field } from "@/components/Field";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { errorMessage } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

/// Counts the dialogs opened. The core files a draft under its dialog's id, and a new draft of
/// the same dialog cancels the one before.
let opened = 0;

type Stage = "describe" | "writing" | "draft";
/// Whether the text can be saved: what the Library will show, or why it cannot be.
type Check = { summary: DraftSummary } | { problem: string } | null;

/// "Create skill" (skill creator spec §4): describe the idea, Claude writes the skill through
/// the person's `claude` CLI, the person edits it or asks for changes, then saves it.
export function CreateSkillDialog({ onClose }: { onClose: () => void }) {
  const qc = useQueryClient();
  const [dialog] = useState(() => `create-skill-${++opened}`);
  const [stage, setStage] = useState<Stage>("describe");
  // Where Writing goes back to when it fails or is cancelled.
  const [from, setFrom] = useState<"describe" | "draft">("describe");
  const [idea, setIdea] = useState("");
  const [model, setModel] = useState("sonnet");
  const [text, setText] = useState("");
  const [change, setChange] = useState("");
  const [check, setCheck] = useState<Check>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [discarding, setDiscarding] = useState(false);
  // The newest request: the answer to an older or a cancelled one is dropped.
  const asked = useRef(0);
  const writing = useRef(false);
  writing.current = stage === "writing";

  // A dialog that goes away while Claude writes ends the CLI.
  useEffect(
    () => () => {
      if (writing.current) void commands.skillDraftCancel(dialog).catch(() => {});
    },
    [dialog],
  );

  useEffect(() => {
    if (stage !== "draft") return;
    let current = true;
    commands.skillCheck(text).then(
      (summary) => current && setCheck({ summary }),
      (e: unknown) => current && setCheck({ problem: errorMessage(e) }),
    );
    return () => {
      current = false;
    };
  }, [stage, text]);

  async function write(back: "describe" | "draft") {
    const n = ++asked.current;
    const request: DraftRequest =
      back === "draft"
        ? { idea, model, current: text, change }
        : { idea, model, current: null, change: null };
    setFrom(back);
    setError(null);
    setStage("writing");
    try {
      const answer = await commands.skillDraft(dialog, request);
      if (n !== asked.current) return;
      setText(answer);
      setChange("");
      setStage("draft");
    } catch (e) {
      if (n !== asked.current) return;
      setError(errorMessage(e));
      setStage(back);
    }
  }

  function stopWriting() {
    asked.current++;
    void commands.skillDraftCancel(dialog).catch(() => {});
    setStage(from);
  }

  async function save() {
    setSaving(true);
    setError(null);
    try {
      await commands.skillCreate(text);
      invalidate(qc, keys.library);
      onClose();
    } catch (e) {
      setError(errorMessage(e));
      setSaving(false);
    }
  }

  function leave() {
    if (stage === "writing") stopWriting();
    onClose();
  }

  function close() {
    if (text.trim() !== "") setDiscarding(true);
    else leave();
  }

  function startOver() {
    setText("");
    setChange("");
    setCheck(null);
    setError(null);
    setStage("describe");
  }

  const ready = check !== null && "summary" in check;
  return (
    <>
      <Dialog open onOpenChange={(next) => !next && close()}>
        <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-3xl">
          <DialogHeader>
            <DialogTitle>Create skill</DialogTitle>
            <DialogDescription>
              Claude writes it through your claude CLI, with no tools. You read it before it is
              saved.
            </DialogDescription>
          </DialogHeader>
          {stage === "describe" && (
            <>
              <Field
                label="What should the skill do?"
                htmlFor="skill-idea"
                hint="A sentence or two is enough."
              >
                <Textarea
                  id="skill-idea"
                  rows={4}
                  value={idea}
                  onChange={(e) => setIdea(e.target.value)}
                />
              </Field>
              <Field label="Model" htmlFor="skill-model">
                <Input
                  id="skill-model"
                  list="skill-models"
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                />
                <datalist id="skill-models">
                  <option value="sonnet" />
                  <option value="opus" />
                  <option value="haiku" />
                </datalist>
              </Field>
            </>
          )}
          {stage === "writing" && (
            <p role="status" className="text-muted-foreground">
              Claude is writing the skill…
            </p>
          )}
          {stage === "draft" && (
            <>
              <Field label="SKILL.md" htmlFor="skill-text">
                <Textarea
                  id="skill-text"
                  rows={16}
                  className="font-mono text-xs"
                  value={text}
                  onChange={(e) => setText(e.target.value)}
                />
              </Field>
              {check &&
                ("summary" in check ? (
                  <p role="status" className="text-xs text-muted-foreground">
                    {`The Library will show ${check.summary.name}: ${check.summary.description}`}
                  </p>
                ) : (
                  <p role="status" className="text-xs text-destructive">
                    {check.problem}
                  </p>
                ))}
              <Field label="What should change?" htmlFor="skill-change">
                <Textarea
                  id="skill-change"
                  rows={2}
                  value={change}
                  onChange={(e) => setChange(e.target.value)}
                />
              </Field>
            </>
          )}
          {error && (
            <p role="alert" className="text-xs text-destructive">
              {error}
            </p>
          )}
          <DialogFooter>
            {stage === "describe" && (
              <Button
                type="button"
                disabled={idea.trim() === "" || model.trim() === ""}
                onClick={() => void write("describe")}
              >
                Write it
              </Button>
            )}
            {stage === "writing" && (
              <Button type="button" variant="outline" onClick={stopWriting}>
                Cancel
              </Button>
            )}
            {stage === "draft" && (
              <>
                <Button type="button" variant="outline" onClick={startOver}>
                  Start over
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  disabled={change.trim() === ""}
                  onClick={() => void write("draft")}
                >
                  Ask for changes
                </Button>
                <Button type="button" disabled={!ready || saving} onClick={() => void save()}>
                  Save to Library
                </Button>
              </>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <AlertDialog open={discarding} onOpenChange={setDiscarding}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Discard this draft?</AlertDialogTitle>
            <AlertDialogDescription>The skill has not been saved.</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep editing</AlertDialogCancel>
            <AlertDialogAction onClick={leave}>Discard</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
