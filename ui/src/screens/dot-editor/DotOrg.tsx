import { Field } from "@/components/Field";
import { Badge } from "@/components/ui/badge";
import { NativeSelect } from "@/components/ui/native-select";
import { orgRemovedText, useDotOrg, useOrgList } from "@/lib/dot-org";

/// The org a new dot belongs to (env vault spec §9): Personal, or one of the orgs joined on
/// this PC. Fixed once the dot is made. With no org joined there is nothing to choose, and
/// nothing shows.
export function OrgPicker(props: { value: string | null; onChange: (org: string | null) => void }) {
  const orgs = useOrgList(true) ?? [];
  if (orgs.length === 0) return null;
  return (
    <Field
      label="Org"
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
/// "org removed" and why the dot can't be saved or run.
export function OrgLine({ orgId }: { orgId: string }) {
  const { name, removed } = useDotOrg(orgId);
  return (
    <div role="group" aria-label="Org" className="flex flex-col gap-1">
      <p className="flex items-center gap-2 text-sm">
        <span>
          Org: <span className="font-medium">{name}</span>
        </span>
        {removed && <Badge variant="destructive">org removed</Badge>}
      </p>
      {removed ? (
        <p className="text-xs text-destructive">{orgRemovedText(orgId)}</p>
      ) : (
        <p className="text-xs text-muted-foreground">
          Its variables come from this org. It was set when the dot was made.
        </p>
      )}
    </div>
  );
}
