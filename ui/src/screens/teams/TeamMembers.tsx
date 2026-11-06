import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";

import { commands } from "@/api/commands";
import type { MemberSpec, Team, TeamMember } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { ErrorNote } from "@/components/ErrorNote";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { PERSONAL, inScope } from "@/lib/scope";

import { teamSpec } from "./format";
import { TeamSection } from "./parts";

/// The team's members (teams spec section 10.1): each dot with its role, which the person
/// edits in place, and "Remove"; then a dot of the team's account and workspace to add.
export function TeamMembers({ team }: { team: Team }) {
  const qc = useQueryClient();
  const dots = useQuery({ queryKey: keys.dots, queryFn: commands.listDots });
  const save = useMutation({
    mutationFn: (members: MemberSpec[]) =>
      commands.teamUpdate(team.id, teamSpec(team, { members })),
    onSuccess: () => invalidate(qc, keys.teams),
    onError: notifyError,
  });
  const members = teamSpec(team).members;
  // A member from elsewhere is listed in the Inbox as a problem, so none is offered.
  const scope = { account: team.org_id ?? PERSONAL, workspace: team.workspace_id };
  const candidates = (dots.data ?? []).filter(
    (dot) => inScope(dot, scope) && !members.some((m) => m.dot_id === dot.id),
  );
  return (
    <TeamSection id="team-members" title="Members">
      {team.members.length === 0 ? (
        <p className="text-muted-foreground">No members yet. Add the dots that work here.</p>
      ) : (
        <ul className="flex flex-col gap-2">
          {team.members.map((member) => (
            <MemberRow
              key={member.dot_id}
              member={member}
              busy={save.isPending}
              onRole={(role) =>
                save.mutate(members.map((m) => (m.dot_id === member.dot_id ? { ...m, role } : m)))
              }
              onRemove={() => save.mutate(members.filter((m) => m.dot_id !== member.dot_id))}
            />
          ))}
        </ul>
      )}
      {dots.isError && <ErrorNote error={dots.error} />}
      {dots.isSuccess && (
        <AddMember
          candidates={candidates}
          busy={save.isPending}
          onAdd={(member) => save.mutate([...members, member])}
        />
      )}
    </TeamSection>
  );
}

/// A member: its name, its role (saved when the field is left or Enter is pressed) and Remove.
function MemberRow(props: {
  member: TeamMember;
  busy: boolean;
  onRole: (role: string) => void;
  onRemove: () => void;
}) {
  const { member } = props;
  const [role, setRole] = useState(member.role);
  useEffect(() => setRole(member.role), [member.role]);
  const commit = () => {
    if (role.trim() !== member.role) props.onRole(role.trim());
  };
  return (
    <li aria-label={member.dot_name} className="flex items-center gap-3 rounded-lg border p-3">
      <span className="min-w-0 flex-1 truncate font-medium">{member.dot_name}</span>
      <Input
        aria-label={`Role of ${member.dot_name}`}
        placeholder="Role, e.g. dev agent"
        className="w-56"
        value={role}
        onChange={(e) => setRole(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") commit();
        }}
      />
      <ConfirmButton
        title={`Remove ${member.dot_name} from the team?`}
        description="The dot stays. A workflow whose steps name it cannot start until you change it."
        confirmLabel="Remove"
        ariaLabel={`Remove ${member.dot_name}`}
        disabled={props.busy}
        onConfirm={props.onRemove}
      >
        Remove
      </ConfirmButton>
    </li>
  );
}

/// A dot to add and its role.
function AddMember(props: {
  candidates: { id: string; name: string }[];
  busy: boolean;
  onAdd: (member: MemberSpec) => void;
}) {
  const [dotId, setDotId] = useState("");
  const [role, setRole] = useState("");
  // A dot added, or gone, is no longer offered.
  const chosen = props.candidates.some((d) => d.id === dotId) ? dotId : "";
  if (props.candidates.length === 0) {
    return (
      <p className="text-xs text-muted-foreground">
        Every dot of the team&apos;s account and workspace is a member.
      </p>
    );
  }
  return (
    <div className="flex flex-wrap items-end gap-2">
      <Field label="Dot" htmlFor="member-dot">
        <NativeSelect id="member-dot" value={chosen} onChange={(e) => setDotId(e.target.value)}>
          <NativeSelectOption value="">Choose a dot</NativeSelectOption>
          {props.candidates.map((dot) => (
            <NativeSelectOption key={dot.id} value={dot.id}>
              {dot.name}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </Field>
      <Field label="Role" htmlFor="member-role">
        <Input
          id="member-role"
          className="w-56"
          placeholder="e.g. dev agent"
          value={role}
          onChange={(e) => setRole(e.target.value)}
        />
      </Field>
      <Button
        size="sm"
        disabled={chosen === "" || props.busy}
        onClick={() => {
          props.onAdd({ dot_id: chosen, role: role.trim() });
          setDotId("");
          setRole("");
        }}
      >
        Add member
      </Button>
    </div>
  );
}
