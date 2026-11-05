import { useQuery } from "@tanstack/react-query";
import type { ReactNode } from "react";

import { commands } from "@/api/commands";
import type { TeamMember } from "@/api/types";
import { Field } from "@/components/Field";
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { keys } from "@/lib/query";

import type { NameOf } from "./labels";

/// A fieldset of the settings sheet, its legend styled like a field's label, with a hint under
/// its controls.
export function Group(props: { legend: string; hint?: ReactNode; children: ReactNode }) {
  return (
    <fieldset className="flex flex-col gap-1.5">
      <legend className="mb-1.5 text-sm leading-none font-medium">{props.legend}</legend>
      {props.children}
      {props.hint && <p className="text-xs text-muted-foreground">{props.hint}</p>}
    </fieldset>
  );
}

/// A member of the team, picked by name and kept by dot id. A dot that is not a member (it
/// left the team) stays shown by its id until another is picked; none picked yet asks for one.
export function MemberSelect(props: {
  id: string;
  label: string;
  hint?: ReactNode;
  members: TeamMember[];
  value: string;
  nameOf: NameOf;
  onChange: (dotId: string) => void;
}) {
  const known = props.members.some((m) => m.dot_id === props.value);
  return (
    <Field label={props.label} htmlFor={props.id} hint={props.hint}>
      <NativeSelect
        id={props.id}
        className="w-full"
        value={props.value}
        onChange={(e) => props.onChange(e.target.value)}
      >
        {props.value === "" && (
          <NativeSelectOption value="" disabled>
            Pick a member
          </NativeSelectOption>
        )}
        {!known && props.value !== "" && (
          <NativeSelectOption value={props.value}>
            {props.nameOf(props.value)} (not a member)
          </NativeSelectOption>
        )}
        {props.members.map((m) => (
          <NativeSelectOption key={m.dot_id} value={m.dot_id}>
            {m.dot_name}
          </NativeSelectOption>
        ))}
      </NativeSelect>
    </Field>
  );
}

/// One checkbox per option, named by its label; `chosen` holds the values ticked. A chosen
/// value no option has (a step that moved, a member who left) stays as a ticked box marked
/// `stale`, so it can be taken out. `empty` shows when there is nothing to choose.
export function ChoiceList(props: {
  legend: string;
  hint?: ReactNode;
  options: { value: string; label: string }[];
  chosen: string[];
  stale: (value: string) => string;
  empty: string;
  onChange: (value: string, on: boolean) => void;
}) {
  const gone = props.chosen.filter((v) => !props.options.some((o) => o.value === v));
  const box = (value: string, label: string, note?: string) => (
    <li key={value}>
      <label className="flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          aria-label={label}
          checked={props.chosen.includes(value)}
          onChange={(e) => props.onChange(value, e.target.checked)}
        />
        {label}
        {note && <span className="text-xs text-destructive">{note}</span>}
      </label>
    </li>
  );
  return (
    <Group legend={props.legend} hint={props.hint}>
      {props.options.length === 0 && gone.length === 0 ? (
        <p className="text-sm text-muted-foreground">{props.empty}</p>
      ) : (
        <ul className="flex flex-col gap-1">
          {props.options.map((o) => box(o.value, o.label))}
          {gone.map((v) => box(v, props.stale(v), "Not allowed here"))}
        </ul>
      )}
    </Group>
  );
}

/// A warning for each of `dotIds` that works in folder mode (teams spec section 8): in a
/// workflow it works in the project's own checkout, not in a worktree of its own. Nothing while
/// the dots are not known.
export function FolderWarning({ dotIds, nameOf }: { dotIds: string[]; nameOf: NameOf }) {
  const dots = useQuery({ queryKey: keys.dots, queryFn: commands.listDots });
  const folder = [...new Set(dotIds)].filter(
    (id) => dots.data?.find((d) => d.id === id)?.workspace_mode === "folder",
  );
  if (folder.length === 0) return null;
  return (
    <div className="flex flex-col gap-1">
      {folder.map((id) => (
        <p
          key={id}
          role="note"
          className="rounded-md border border-needs-border bg-needs-surface p-2 text-xs text-needs"
        >
          {nameOf(id)} works in the folder itself: in a workflow it changes the project's own
          checkout, and leaves no branch to check or deploy.
        </p>
      ))}
    </div>
  );
}
