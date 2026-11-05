import { useMutation, useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen, Plus } from "lucide-react";
import { useState } from "react";

import { commands } from "@/api/commands";
import type { Project, ProjectSpec, Team } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
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
import { Input } from "@/components/ui/input";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { ItemRow } from "@/screens/library/parts";

import { TeamSection } from "./parts";

/// The team's projects (teams spec section 10.1): each local git repo with its remote and
/// default branch, to open, edit or delete; and "Add project".
export function TeamProjects({ team }: { team: Team }) {
  const [editing, setEditing] = useState<Project | "new" | null>(null);
  return (
    <TeamSection
      id="team-projects"
      title="Projects"
      actions={
        <Button size="sm" variant="outline" onClick={() => setEditing("new")}>
          <Plus aria-hidden /> Add project
        </Button>
      }
    >
      {team.projects.length === 0 ? (
        <p className="text-muted-foreground">
          No projects yet. A project is a git repo on this PC that the team works in.
        </p>
      ) : (
        <ul className="flex flex-col gap-2">
          {team.projects.map((project) => (
            <ProjectRow key={project.id} project={project} onEdit={() => setEditing(project)} />
          ))}
        </ul>
      )}
      {editing !== null && (
        <ProjectDialog
          teamId={team.id}
          project={editing === "new" ? null : editing}
          onClose={() => setEditing(null)}
        />
      )}
    </TeamSection>
  );
}

function ProjectRow({ project, onEdit }: { project: Project; onEdit: () => void }) {
  const qc = useQueryClient();
  const name = project.name;
  const remove = useMutation({
    mutationFn: () => commands.projectDelete(project.id),
    onSuccess: () => invalidate(qc, keys.teams),
    onError: notifyError,
  });
  return (
    <ItemRow
      label={name}
      actions={
        <>
          <Button
            size="sm"
            variant="ghost"
            aria-label={`Open the folder of ${name}`}
            title="Open the folder"
            onClick={() => commands.openProjectFolder(project.id).catch(notifyError)}
          >
            <FolderOpen aria-hidden />
          </Button>
          <Button size="sm" variant="outline" aria-label={`Edit ${name}`} onClick={onEdit}>
            Edit
          </Button>
          <ConfirmButton
            title={`Delete ${name}?`}
            description="Its folder and git repo stay as they are: only the team forgets it. A schedule or webhook that names it cannot start until you change it."
            confirmLabel="Delete project"
            ariaLabel={`Delete ${name}`}
            disabled={remove.isPending}
            onConfirm={() => remove.mutate()}
          >
            Delete
          </ConfirmButton>
        </>
      }
    >
      <span className="font-medium">{name}</span>
      <span className="font-mono text-xs break-all text-muted-foreground">
        {project.repo_path}
      </span>
      <span className="text-xs text-muted-foreground">
        Remote {project.remote} ·{" "}
        {project.default_branch === null
          ? "default branch found by git"
          : `default branch ${project.default_branch}`}
      </span>
    </ItemRow>
  );
}

/// The last part of a folder's path: the name a project picked from it starts with.
function folderName(path: string): string {
  return path.replace(/[\\/]+$/, "").split(/[\\/]/).at(-1) ?? "";
}

/// "Add project" and "Edit": the name, the repo folder (picked or typed), the remote and the
/// default branch. The core refuses a folder that is not a git repo, with its reason.
function ProjectDialog(props: { teamId: string; project: Project | null; onClose: () => void }) {
  const qc = useQueryClient();
  const { project } = props;
  const [name, setName] = useState(project?.name ?? "");
  const [folder, setFolder] = useState(project?.repo_path ?? "");
  const [remote, setRemote] = useState(project?.remote ?? "origin");
  const [branch, setBranch] = useState(project?.default_branch ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  function pick() {
    open({ directory: true, title: "Choose the project's git repo" }).then((picked) => {
      if (typeof picked !== "string") return;
      setFolder(picked);
      if (name.trim() === "") setName(folderName(picked));
    }, notifyError);
  }

  async function save() {
    const spec: ProjectSpec = {
      name: name.trim(),
      repo_path: folder.trim(),
      default_branch: branch.trim() === "" ? null : branch.trim(),
      remote: remote.trim() === "" ? "origin" : remote.trim(),
    };
    setBusy(true);
    setError(undefined);
    try {
      if (project) await commands.projectUpdate(project.id, spec);
      else await commands.projectAdd(props.teamId, spec);
      invalidate(qc, keys.teams);
      props.onClose();
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  }

  const missing =
    name.trim() === ""
      ? "Give the project a name first."
      : folder.trim() === ""
        ? "Choose its folder first."
        : undefined;
  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{project ? `Edit ${project.name}` : "Add project"}</DialogTitle>
          <DialogDescription>
            A git repo on this PC. Each workflow run works in worktrees of it, on its own
            branches.
          </DialogDescription>
        </DialogHeader>
        <Field label="Name" htmlFor="project-name">
          <Input id="project-name" value={name} onChange={(e) => setName(e.target.value)} />
        </Field>
        <Field label="Folder" htmlFor="project-folder">
          <div className="flex gap-2">
            <Input
              id="project-folder"
              className="font-mono text-xs"
              value={folder}
              onChange={(e) => setFolder(e.target.value)}
            />
            <Button type="button" variant="outline" onClick={pick}>
              Choose…
            </Button>
          </div>
        </Field>
        <Field
          label="Remote"
          htmlFor="project-remote"
          hint="Where push and pull request steps push."
        >
          <Input id="project-remote" value={remote} onChange={(e) => setRemote(e.target.value)} />
        </Field>
        <Field
          label="Default branch"
          htmlFor="project-branch"
          hint="Leave it empty to use the repo's own, as git finds it."
        >
          <Input
            id="project-branch"
            placeholder="main"
            value={branch}
            onChange={(e) => setBranch(e.target.value)}
          />
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
            onClick={() => void save()}
          >
            {project ? "Save" : "Add project"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
