import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useId, useRef, useState } from "react";
import { Link, useNavigate } from "react-router";
import { toast } from "sonner";

import { commands } from "@/api/commands";
import type { SetupProposal, SetupRequest, SetupRound } from "@/api/types";
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
import { Textarea } from "@/components/ui/textarea";
import { nameError, scheduleError } from "@/lib/dot-rules";
import { errorMessage } from "@/lib/notify";
import { invalidateDots, keys } from "@/lib/query";
import { PERSONAL } from "@/lib/scope";
import { SetupCard } from "@/screens/setup/SetupCard";

/// Counts the setup views opened. The core files a request under its view's id, and a newer
/// request of the same view cancels the one before; two views never cancel each other.
let opened = 0;

type Stage = "describe" | "asking" | "questions" | "card";
/// Where a request goes back to when it fails or is cancelled.
type Back = Exclude<Stage, "asking">;

const MAX_TEXT = 4000;

/// The confirmation before a proposed dot is thrown away: by Start over, or by closing the New
/// dot dialog. `action` names what it does.
export function DiscardConfirm(props: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  action: string;
  onConfirm: () => void;
}) {
  return (
    <AlertDialog open={props.open} onOpenChange={props.onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Discard the proposed dot?</AlertDialogTitle>
          <AlertDialogDescription>It has not been made yet.</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Keep it</AlertDialogCancel>
          <AlertDialogAction onClick={props.onConfirm}>{props.action}</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/// The setup agent's view (setup agent spec §5): describe a task, answer at most two short
/// question rounds, edit the proposed dot on one card, and Run setup. Used on the empty home
/// and in the New dot dialog; `onDone` is called once the dot is made.
export function SetupFlow(props: {
  onDone?: () => void;
  title?: string;
  /// Told whether a proposed dot is on hand, so that a dialog can confirm before closing.
  onProposal?: (has: boolean) => void;
  /// Told whether Run setup is making the dot, so that a dialog stays open meanwhile.
  onApplying?: (applying: boolean) => void;
}) {
  const title = props.title ?? "What should your first dot do?";
  const qc = useQueryClient();
  const navigate = useNavigate();
  const ids = useId();
  const [id] = useState(() => `setup-${++opened}`);
  const [stage, setStage] = useState<Stage>("describe");
  const [back, setBack] = useState<Back>("describe");
  const [description, setDescription] = useState("");
  // The question rounds answered before the first proposal.
  const [rounds, setRounds] = useState<SetupRound[]>([]);
  const [questions, setQuestions] = useState<string[]>([]);
  const [answers, setAnswers] = useState<string[]>([]);
  const [proposal, setProposal] = useState<SetupProposal | null>(null);
  const [root, setRoot] = useState("");
  const [change, setChange] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [applyError, setApplyError] = useState<string | null>(null);
  const [applying, setApplying] = useState(false);
  const [confirming, setConfirming] = useState(false);
  // The newest request: the answer to an older or a cancelled one is dropped.
  const asked = useRef(0);
  const asking = useRef(false);
  asking.current = stage === "asking";
  // Whether the claude CLI was found, to say what a failure most likely means.
  const status = useQuery({ queryKey: keys.status, queryFn: commands.status }).data;
  const noCli = status !== undefined && status.claude.program === null;

  const has = proposal !== null;
  const { onProposal } = props;
  useEffect(() => onProposal?.(has), [has, onProposal]);
  const { onApplying } = props;
  useEffect(() => onApplying?.(applying), [applying, onApplying]);

  // A view that goes away while the agent works ends the CLI.
  useEffect(
    () => () => {
      if (asking.current) void commands.setupCancel(id).catch(() => {});
    },
    [id],
  );

  async function ask(request: SetupRequest, from: Back, answered: SetupRound[]) {
    const n = ++asked.current;
    setBack(from);
    setError(null);
    setApplyError(null);
    setStage("asking");
    try {
      const answer = await commands.setupAsk(id, request);
      if (n !== asked.current) return;
      setRounds(answered);
      if (answer.kind === "questions") {
        setQuestions(answer.questions);
        setAnswers(answer.questions.map(() => ""));
        setStage("questions");
      } else {
        setProposal(answer.proposal);
        setRoot(answer.new_dot_root);
        setChange("");
        setStage("card");
      }
    } catch (e) {
      if (n !== asked.current) return;
      setError(errorMessage(e));
      setStage(from);
    }
  }

  const base = { description: description.trim(), current: null, change: null };

  function start() {
    void ask({ ...base, rounds: [], skip: false }, "describe", []);
  }

  function answer() {
    const answered = [...rounds, { questions, answers: answers.map((a) => a.trim()) }];
    void ask({ ...base, rounds: answered, skip: false }, "questions", answered);
  }

  function skip() {
    void ask({ ...base, rounds, skip: true }, "questions", rounds);
  }

  function askForChanges() {
    if (!proposal) return;
    void ask(
      { ...base, rounds, current: proposal, change: change.trim(), skip: false },
      "card",
      rounds,
    );
  }

  function cancel() {
    asked.current++;
    void commands.setupCancel(id).catch(() => {});
    setStage(back);
  }

  function startOver() {
    asked.current++;
    setConfirming(false);
    setDescription("");
    setRounds([]);
    setQuestions([]);
    setAnswers([]);
    setProposal(null);
    setChange("");
    setError(null);
    setApplyError(null);
    setStage("describe");
  }

  async function runSetup() {
    if (!proposal) return;
    setApplying(true);
    setError(null);
    setApplyError(null);
    try {
      // The dot goes to the account and workspace in view when Run setup is pressed.
      const scope = await qc.fetchQuery({ queryKey: keys.scope, queryFn: commands.scopeGet });
      const org = scope.account === PERSONAL ? null : scope.account;
      const workspace = scope.workspace === "default" ? null : scope.workspace;
      const applied = await commands.setupApply(proposal, org, workspace);
      invalidateDots(qc);
      qc.setQueryData(keys.dot(applied.dot.id), applied.dot);
      // The home finds the dot it opens in the overview: without the new dot there, it would
      // send the person back to another one.
      await qc.refetchQueries({ queryKey: keys.overview });
      if (applied.run_error !== null) {
        toast.error(`The dot was created but its first run did not start: ${applied.run_error}`);
      }
      // What the check at Run setup changed, which the card did not show.
      if (applied.notes.trim() !== "") toast(applied.notes);
      props.onDone?.();
      void navigate(`/home/${applied.dot.id}`);
    } catch (e) {
      setApplyError(errorMessage(e));
      setApplying(false);
    }
  }

  const ready =
    proposal !== null &&
    nameError(proposal.name) === undefined &&
    proposal.instructions.trim() !== "" &&
    scheduleError(proposal.schedule) === undefined;
  return (
    <div data-setup-flow tabIndex={-1} className="flex w-full flex-col gap-4 outline-none">
      {stage === "describe" && (
        <>
          <h2 id={`${ids}-title`} className="text-lg font-medium">
            {title}
          </h2>
          <Textarea
            aria-labelledby={`${ids}-title`}
            data-setup-composer
            rows={4}
            maxLength={MAX_TEXT}
            placeholder="For example: every weekday at 9, read my Downloads folder and tell me what's new"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
        </>
      )}
      {stage === "asking" && (
        <p role="status" className="text-muted-foreground">
          Setting up...
        </p>
      )}
      {stage === "questions" &&
        questions.map((question, i) => (
          <Field key={i} label={question} htmlFor={`${ids}-answer-${i}`}>
            <Textarea
              id={`${ids}-answer-${i}`}
              rows={2}
              maxLength={MAX_TEXT}
              value={answers[i] ?? ""}
              onChange={(e) =>
                setAnswers((all) => all.map((a, j) => (j === i ? e.target.value : a)))
              }
            />
          </Field>
        ))}
      {stage === "card" && proposal && (
        <>
          <SetupCard proposal={proposal} newDotRoot={root} onChange={setProposal} />
          <Field label="What should change?" htmlFor={`${ids}-change`}>
            <Textarea
              id={`${ids}-change`}
              rows={2}
              maxLength={MAX_TEXT}
              value={change}
              onChange={(e) => setChange(e.target.value)}
            />
          </Field>
        </>
      )}
      {(error ?? applyError) && (
        <div role="alert" className="flex flex-col gap-1 text-sm text-destructive">
          <p>{error ?? applyError}</p>
          {error && noCli && (
            <p>Dots uses your Claude Code CLI. Install it and sign in, then try again.</p>
          )}
        </div>
      )}
      <div className="flex flex-wrap items-center gap-2">
        {stage === "describe" && (
          <Button type="button" disabled={description.trim() === ""} onClick={start}>
            Set it up
          </Button>
        )}
        {stage === "asking" && (
          <Button type="button" variant="outline" onClick={cancel}>
            Cancel
          </Button>
        )}
        {stage === "questions" && (
          <>
            <Button type="button" onClick={answer}>
              Continue
            </Button>
            <Button type="button" variant="outline" onClick={skip}>
              Skip, just propose
            </Button>
          </>
        )}
        {stage === "card" && (
          <>
            <Button
              type="button"
              disabled={!ready || applying}
              onClick={() => void runSetup()}
            >
              Run setup
            </Button>
            <Button
              type="button"
              variant="outline"
              disabled={change.trim() === "" || applying}
              onClick={askForChanges}
            >
              Ask for changes
            </Button>
          </>
        )}
        {(stage === "questions" || stage === "card") && (
          <Button
            type="button"
            variant="ghost"
            disabled={applying}
            onClick={() => (stage === "card" ? setConfirming(true) : startOver())}
          >
            Start over
          </Button>
        )}
      </div>
      <Link
        to="/dots/new"
        className="self-start text-sm text-muted-foreground underline-offset-4 hover:underline"
      >
        Fill in the form instead
      </Link>
      <DiscardConfirm
        open={confirming}
        onOpenChange={setConfirming}
        action="Start over"
        onConfirm={startOver}
      />
    </div>
  );
}
