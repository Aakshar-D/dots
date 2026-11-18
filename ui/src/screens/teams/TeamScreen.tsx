import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useNavigate, useParams } from "react-router";

import { commands } from "@/api/commands";
import type { Team } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { ErrorNote } from "@/components/ErrorNote";
import { Field } from "@/components/Field";
import { Page } from "@/components/Page";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { plural } from "@/screens/library/parts";

import { teamSpec } from "./format";
import { TeamMembers } from "./TeamMembers";
import { TeamProjects } from "./TeamProjects";
import { TeamRuns } from "./TeamRuns";
import { TeamWorkflows } from "./TeamWorkflows";
import { TeamWaiting } from "./WaitingRuns";

/// A team's page (teams spec section 10.1): its name and description, the runs that wait for
/// the person, members, projects, workflows and recent workflow runs, and "Delete team".
export function TeamScreen() {
  const { teamId = "" } = useParams();
  const team = useQuery({ queryKey: keys.team(teamId), queryFn: () => commands.teamGet(teamId) });
  const workflows = useQuery({
    queryKey: keys.workflows(teamId),
    queryFn: () => commands.workflowsList(teamId),
  });
  if (team.isPending) return <Page title="Team" />;
  if (team.isError) {
    return (
      <Page title="Team">
        <ErrorNote error={team.error} />
      </Page>
    );
  }
  const inFlight = (workflows.data ?? []).reduce((n, w) => n + w.runs_in_flight, 0);
  return (
    <Page title={team.data.name} actions={<DeleteTeam team={team.data} inFlight={inFlight} />}>
      <TeamDetails team={team.data} />
      <TeamWaiting teamId={teamId} />
      <TeamMembers team={team.data} />
      <TeamProjects team={team.data} />
      {workflows.isError ? (
        <ErrorNote error={workflows.error} />
      ) : (
        <TeamWorkflows team={team.data} workflows={workflows.data} />
      )}
      <TeamRuns teamId={teamId} workflows={workflows.data ?? []} />
    </Page>
  );
}

/// The team's name and description, saved together.
function TeamDetails({ team }: { team: Team }) {
  const qc = useQueryClient();
  const [name, setName] = useState(team.name);
  const [description, setDescription] = useState(team.description);
  // What the team says now, when it changes elsewhere.
  useEffect(() => {
    setName(team.name);
    setDescription(team.description);
  }, [team.name, team.description]);
  const save = useMutation({
    mutationFn: () =>
      commands.teamUpdate(
        team.id,
        teamSpec(team, { name: name.trim(), description: description.trim() }),
      ),
    onSuccess: () => invalidate(qc, keys.teams),
    onError: notifyError,
  });
  const changed = name.trim() !== team.name || description.trim() !== team.description;
  return (
    <div className="flex max-w-2xl flex-col gap-3">
      <Field label="Name" htmlFor="team-name">
        <Input id="team-name" value={name} onChange={(e) => setName(e.target.value)} />
      </Field>
      <Field label="Description" htmlFor="team-description">
        <Textarea
          id="team-description"
          rows={2}
          value={description}
          onChange={(e) => setDescription(e.target.value)}
        />
      </Field>
      <div>
        <Button
          size="sm"
          disabled={!changed || name.trim() === "" || save.isPending}
          onClick={() => save.mutate()}
        >
          Save
        </Button>
      </div>
    </div>
  );
}

/// "Delete team": asks first, saying that its runs in flight are stopped, then goes back to the
/// list of teams.
function DeleteTeam({ team, inFlight }: { team: Team; inFlight: number }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const remove = useMutation({
    mutationFn: () => commands.teamDelete(team.id),
    onSuccess: () => {
      navigate("/teams");
      invalidate(qc, keys.teams);
      invalidate(qc, ["workflow-runs"]);
      invalidate(qc, keys.inbox);
    },
    onError: notifyError,
  });
  return (
    <ConfirmButton
      title={`Delete ${team.name}?`}
      description={
        `Its projects, workflows and workflow runs are deleted with it. Runs in flight are ` +
        `stopped first${inFlight > 0 ? `: ${plural(inFlight, "run")} now` : ""}. The sols and ` +
        `the project folders stay.`
      }
      confirmLabel="Delete team"
      disabled={remove.isPending}
      onConfirm={() => remove.mutate()}
    >
      Delete team
    </ConfirmButton>
  );
}
