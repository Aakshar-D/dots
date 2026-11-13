import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Plus, UsersRound } from "lucide-react";
import { useState } from "react";
import { Link, useNavigate } from "react-router";

import { commands } from "@/api/commands";
import { EmptyState } from "@/components/EmptyState";
import { ErrorNote } from "@/components/ErrorNote";
import { Field } from "@/components/Field";
import { Page } from "@/components/Page";
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
import { PERSONAL, useCurrentScope } from "@/lib/scope";
import { ItemRow, plural } from "@/screens/library/parts";

/// The Teams screen (teams spec section 10.1): every team with its members and projects, each
/// opening its page, and "New team".
export function TeamsScreen() {
  const teams = useQuery({ queryKey: keys.teams, queryFn: commands.teamsList });
  const [creating, setCreating] = useState(false);
  const actions = (
    <Button size="sm" onClick={() => setCreating(true)}>
      <Plus aria-hidden /> New team
    </Button>
  );
  return (
    <Page title="Teams" actions={actions}>
      {teams.isError && <ErrorNote error={teams.error} />}
      {teams.data?.length === 0 && (
        <EmptyState icon={UsersRound}>
          No teams yet. A team is sols that work on a project together, step by step, as a
          workflow says: make one with New team.
        </EmptyState>
      )}
      {(teams.data ?? []).length > 0 && (
        <ul className="flex flex-col gap-2">
          {teams.data!.map((team) => (
            <ItemRow key={team.id} label={team.name}>
              <Link to={`/teams/${team.id}`} className="font-medium hover:underline">
                {team.name}
              </Link>
              {team.description && (
                <span className="text-xs text-muted-foreground">{team.description}</span>
              )}
              <span className="text-xs text-muted-foreground">
                {plural(team.members.length, "member")} · {plural(team.projects.length, "project")}
              </span>
            </ItemRow>
          ))}
        </ul>
      )}
      {creating && <NewTeamDialog onClose={() => setCreating(false)} />}
    </Page>
  );
}

/// "New team": its name and description, made in the account and workspace the person works
/// in now, then opened.
function NewTeamDialog(props: { onClose: () => void }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const scope = useCurrentScope();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  async function create() {
    if (!scope) return;
    setBusy(true);
    setError(undefined);
    try {
      const team = await commands.teamCreate({
        name: name.trim(),
        description: description.trim(),
        members: [],
        org_id: scope.account === PERSONAL ? null : scope.account,
        workspace_id: scope.workspace,
      });
      invalidate(qc, keys.teams);
      navigate(`/teams/${team.id}`);
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  }

  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>New team</DialogTitle>
          <DialogDescription>
            Add its members, projects and workflows on its page next.
          </DialogDescription>
        </DialogHeader>
        <Field label="Name" htmlFor="team-name">
          <Input id="team-name" value={name} onChange={(e) => setName(e.target.value)} />
        </Field>
        <Field label="Description" htmlFor="team-description">
          <Textarea
            id="team-description"
            rows={3}
            value={description}
            onChange={(e) => setDescription(e.target.value)}
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
            disabled={busy || !scope || name.trim() === ""}
            title={name.trim() === "" ? "Give the team a name first." : undefined}
            onClick={() => void create()}
          >
            Create team
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
