import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { commands } from "@/api/commands";
import type { Dot } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import { Field } from "@/components/Field";
import { Badge } from "@/components/ui/badge";
import { NativeSelect } from "@/components/ui/native-select";
import { orgRemovedText, useDotOrg, useOrgList } from "@/lib/dot-org";
import { invalidateDots, keys } from "@/lib/query";
import { PERSONAL, useWorkspaces } from "@/lib/scope";

const DEFAULT_WORKSPACE = "default";

/// The org a new dot belongs to (env vault spec §9): Personal, or one of the orgs joined on
/// this PC. Fixed once the dot is made. With no org joined there is nothing to choose, and
/// nothing shows.
export function OrgPicker(props: { value: string | null; onChange: (org: string | null) => void }) {
  const orgs = useOrgList(true) ?? [];
  if (orgs.length === 0) return null;
  return (
    <Field
      label="Account"
      htmlFor="dot-org"
      hint="Where the dot's variables come from. It can't be changed once the dot is made."
    >
      <NativeSelect
        id="dot-org"
        value={props.value ?? ""}
        onChange={(e) => props.onChange(e.target.value === "" ? null : e.target.value)}
      >
        <option value="">Personal</option>
        {orgs.map((org) => (
          <option key={org.id} value={org.id}>
            {org.name}
          </option>
        ))}
      </NativeSelect>
    </Field>
  );
}

/// The org of a dot that was made in one, as read-only text; when the org is no longer joined,
/// "account removed" and why the dot can't be saved or run.
export function OrgLine({ orgId }: { orgId: string }) {
  const { name, removed } = useDotOrg(orgId);
  return (
    <div role="group" aria-label="Account" className="flex flex-col gap-1">
      <p className="flex items-center gap-2 text-sm">
        <span>
          Account: <span className="font-medium">{name}</span>
        </span>
        {removed && <Badge variant="destructive">account removed</Badge>}
      </p>
      {removed ? (
        <p className="text-xs text-destructive">{orgRemovedText(orgId)}</p>
      ) : (
        <p className="text-xs text-muted-foreground">
          Its variables come from this account. It was set when the dot was made.
        </p>
      )}
    </div>
  );
}

/// The workspaces of `account` (`"personal"` or an org id) a dot can be in, when it has more
/// than Default to choose from; `undefined` otherwise, and nothing shows.
function useChoices(account: string) {
  const all = useWorkspaces(account) ?? [];
  return all.length > 1 ? all : undefined;
}

/// The workspace a new dot starts in (spec: accounts workspaces), among the visible ones of its
/// account. `null` stands for the default one.
export function WorkspacePicker(props: {
  account: string;
  value: string | null;
  onChange: (workspace: string | null) => void;
}) {
  const choices = useChoices(props.account);
  if (!choices) return null;
  return (
    <Field label="Workspace" htmlFor="dot-workspace" hint="Where the dot is listed and works.">
      <NativeSelect
        id="dot-workspace"
        value={props.value ?? DEFAULT_WORKSPACE}
        onChange={(e) =>
          props.onChange(e.target.value === DEFAULT_WORKSPACE ? null : e.target.value)
        }
      >
        {choices.map((w) => (
          <option key={w.id} value={w.id}>
            {w.name}
          </option>
        ))}
      </NativeSelect>
    </Field>
  );
}

/// The workspace of a stored dot; changing it moves the dot at once (`dotMove`), and the core's
/// refusal is shown below. The account stays fixed.
export function WorkspaceMove({ dot }: { dot: Dot }) {
  const qc = useQueryClient();
  const choices = useChoices(dot.org_id ?? PERSONAL);
  const [placed, setPlaced] = useState(dot.workspace_id);
  const move = useMutation({
    mutationFn: (workspace: string) => commands.moveDot(dot.id, workspace),
    onSuccess: (moved) => {
      setPlaced(moved.workspace_id);
      qc.setQueryData(keys.dot(dot.id), moved);
      invalidateDots(qc);
    },
  });
  if (!choices) return null;
  return (
    <Field label="Workspace" htmlFor="dot-workspace" hint="Moving it takes effect at once.">
      <NativeSelect
        id="dot-workspace"
        value={placed}
        disabled={move.isPending}
        title={move.isPending ? "Moving the dot" : undefined}
        onChange={(e) => move.mutate(e.target.value)}
      >
        {choices.map((w) => (
          <option key={w.id} value={w.id}>
            {w.name}
          </option>
        ))}
      </NativeSelect>
      {move.isError && <ErrorNote error={move.error} />}
    </Field>
  );
}
