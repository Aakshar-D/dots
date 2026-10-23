import { useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { Plus, Sparkles } from "lucide-react";
import { useEffect, useState } from "react";

import { commands } from "@/api/commands";
import type { ScannedSkill } from "@/api/types";
import { EmptyState } from "@/components/EmptyState";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

import { CreateSkillDialog } from "./CreateSkillDialog";
import { isDeclined, ItemRow, plural, RemoveButton, UsedBy, type SkillEntry } from "./parts";

type Scan = { folder: string; skills: ScannedSkill[] };

/// The skills found under a folder, to tick. A name the Library cannot take, or has already, is
/// shown with the reason and cannot be ticked.
function AddSkillsDialog(props: {
  scan: Scan;
  existing: Set<string>;
  onChoose: () => void;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const { skills } = props.scan;
  const problem = (s: ScannedSkill) =>
    s.name_error ?? (props.existing.has(s.name) ? "Already in the Library." : null);
  const [unticked, setUnticked] = useState<Set<string>>(new Set());
  const chosen = skills.filter((s) => !problem(s) && !unticked.has(s.path));
  const [state, setState] = useState<{ busy: boolean; error?: string; declined?: boolean }>({
    busy: false,
  });

  // Another folder: what happened with the last one no longer applies.
  useEffect(() => setState({ busy: false }), [props.scan]);

  async function add() {
    setState({ busy: true });
    try {
      for (const skill of chosen) {
        await commands.libraryAdd(
          skill.name,
          { kind: "skill", config: { path: skill.path } },
          null,
        );
        // What was added stays added, whatever comes next.
        invalidate(qc, keys.library);
      }
      props.onClose();
    } catch (e) {
      setState(isDeclined(e) ? { busy: false, declined: true } : { busy: false, error: errorMessage(e) });
    }
  }

  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Add skills</DialogTitle>
          <DialogDescription className="font-mono">{props.scan.folder}</DialogDescription>
        </DialogHeader>
        {skills.length === 0 ? (
          <p className="text-muted-foreground">No SKILL.md found in that folder.</p>
        ) : (
          <ul className="flex flex-col gap-1.5">
            {skills.map((skill) => {
              const why = problem(skill);
              return (
                <li key={skill.path} className="flex items-start gap-2 rounded-md border p-2">
                  <input
                    type="checkbox"
                    className="mt-1"
                    aria-label={skill.name}
                    disabled={why !== null}
                    checked={why === null && !unticked.has(skill.path)}
                    onChange={(e) => {
                      const next = new Set(unticked);
                      if (e.target.checked) next.delete(skill.path);
                      else next.add(skill.path);
                      setUnticked(next);
                    }}
                  />
                  <div className="flex min-w-0 flex-col">
                    <span className="font-medium">{skill.name}</span>
                    {skill.description && (
                      <span className="text-xs text-muted-foreground">{skill.description}</span>
                    )}
                    <span className="truncate font-mono text-xs text-muted-foreground">
                      {skill.path}
                    </span>
                    {why && <span className="text-xs text-destructive">{why}</span>}
                  </div>
                </li>
              );
            })}
          </ul>
        )}
        {state.error && (
          <p role="alert" className="text-xs text-destructive">
            {state.error}
          </p>
        )}
        {state.declined && (
          <p role="status" className="text-xs text-muted-foreground">
            You declined. Nothing more was added.
          </p>
        )}
        <DialogFooter>
          <Button type="button" variant="outline" onClick={props.onChoose}>
            Choose another folder
          </Button>
          {chosen.length > 0 && (
            <Button type="button" disabled={state.busy} onClick={() => void add()}>
              {`Add ${plural(chosen.length, "skill")}`}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/// The skills tab. "Add skill" picks a folder: one skill, or a folder with several below it.
/// "Create skill" has Claude write one from an idea.
export function SkillsTab({ entries }: { entries: SkillEntry[] }) {
  const [scan, setScan] = useState<Scan | null>(null);
  const [creating, setCreating] = useState(false);

  async function choose() {
    try {
      const folder = await open({ directory: true, title: "Choose a skill or a folder of skills" });
      if (typeof folder !== "string") return;
      setScan({ folder, skills: await commands.libraryScanSkills(folder) });
    } catch (e) {
      notifyError(e);
    }
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-3">
        <p className="flex-1 text-xs text-muted-foreground">
          Folders with a SKILL.md. A dot that uses a skill can read it when the task calls for it.
        </p>
        <Button size="sm" variant="outline" onClick={() => setCreating(true)}>
          <Sparkles aria-hidden /> Create skill
        </Button>
        <Button size="sm" onClick={() => void choose()}>
          <Plus aria-hidden /> Add skill
        </Button>
      </div>
      {entries.length === 0 ? (
        <EmptyState icon={Sparkles}>No skills yet.</EmptyState>
      ) : (
        <ul className="flex flex-col gap-2">
          {entries.map((entry) => (
            <ItemRow key={entry.id} label={entry.name} actions={<RemoveButton entry={entry} />}>
              <span className="font-medium">{entry.name}</span>
              {entry.skill?.description && (
                <span className="text-xs">{entry.skill.description}</span>
              )}
              <span className="truncate font-mono text-xs text-muted-foreground">
                {entry.config.path}
              </span>
              {entry.problem && <span className="text-xs text-destructive">{entry.problem}</span>}
              <UsedBy dots={entry.used_by} />
            </ItemRow>
          ))}
        </ul>
      )}
      {creating && <CreateSkillDialog onClose={() => setCreating(false)} />}
      {scan && (
        <AddSkillsDialog
          scan={scan}
          existing={new Set(entries.map((e) => e.name))}
          onChoose={() => void choose()}
          onClose={() => setScan(null)}
        />
      )}
    </div>
  );
}
