import { useQuery } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { X } from "lucide-react";
import { useId, type ReactNode } from "react";

import { commands } from "@/api/commands";
import { Preset, type SetupProposal, type Uses } from "@/api/types";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { Textarea } from "@/components/ui/textarea";
import { PRESET_SENTENCES, folderName, nameError, scheduleError, scheduleOf } from "@/lib/dot-rules";
import { errorMessage, notifyError } from "@/lib/notify";
import { keys } from "@/lib/query";
import { useDebounced } from "@/lib/use-debounced";
import { ScheduleEditor } from "@/screens/dot-editor/ScheduleEditor";

/// The presets the setup card offers, each in one plain sentence (setup agent spec §6).
const PRESETS: [Preset, string, string][] = [
  ["read-only", "Read-only", PRESET_SENTENCES["read-only"]],
  ["sandboxed", "Sandboxed", PRESET_SENTENCES.sandboxed],
  ["trusted", "Trusted", PRESET_SENTENCES.trusted],
];

/// An extra of the proposal in plain words.
export function extraLine(extra: string): string {
  if (extra === "web") return "Read websites and search the web without asking";
  return `Use ${extra.replace(/^mcp:/, "")} without asking`;
}

const USES: (keyof Uses)[] = ["mcp_servers", "skills", "plugins", "context", "apps"];

function Part(props: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-1.5">
      <h3 className="text-sm font-medium">{props.title}</h3>
      {props.children}
    </section>
  );
}

/// The setup card (setup agent spec §8): the proposed dot, every field editable in place.
/// `newDotRoot` is where Dots makes the dot's folder when the proposal names none.
export function SetupCard(props: {
  proposal: SetupProposal;
  newDotRoot: string;
  onChange: (proposal: SetupProposal) => void;
}) {
  const { proposal, onChange } = props;
  const id = useId();
  const set = <K extends keyof SetupProposal>(key: K, value: SetupProposal[K]) =>
    onChange({ ...proposal, [key]: value });

  // The core computes the next runs and says what is wrong with a schedule, as in the editor.
  const schedule = scheduleOf(useDebounced(proposal.schedule, 300));
  const sendable = schedule !== null && scheduleError(schedule) === undefined;
  const preview = useQuery({
    queryKey: keys.schedulePreview(schedule),
    queryFn: () => (schedule ? commands.previewSchedule(schedule, 3) : []),
    enabled: sendable,
  });
  const scheduleProblem =
    scheduleError(proposal.schedule) ??
    (sendable && preview.isError ? errorMessage(preview.error) : undefined);

  function chooseFolder() {
    open({ directory: true, title: "Choose the folder the dot works in" }).then((picked) => {
      if (typeof picked === "string") set("folder", picked);
    }, notifyError);
  }

  function removeUse(kind: keyof Uses, item: string) {
    const uses = { ...proposal.uses, [kind]: proposal.uses[kind].filter((i) => i !== item) };
    // A server that is no longer used needs no permission either.
    const extras =
      kind === "mcp_servers" ? proposal.extras.filter((e) => e !== `mcp:${item}`) : proposal.extras;
    onChange({ ...proposal, uses, extras });
  }

  const preset = PRESETS.find(([p]) => p === proposal.preset);
  const used = USES.flatMap((kind) => proposal.uses[kind].map((item) => [kind, item] as const));
  return (
    <div className="flex flex-col gap-4">
      <Field label="Name" htmlFor={`${id}-name`} error={nameError(proposal.name)}>
        <Input
          id={`${id}-name`}
          className="text-base font-medium"
          value={proposal.name}
          onChange={(e) => set("name", e.target.value)}
        />
      </Field>
      {proposal.notes.trim() !== "" && (
        <p className="text-sm text-muted-foreground">{proposal.notes}</p>
      )}
      <Field
        label="What it does"
        htmlFor={`${id}-instructions`}
        error={proposal.instructions.trim() === "" ? "Say what the dot should do." : undefined}
      >
        <Textarea
          id={`${id}-instructions`}
          rows={10}
          className="field-sizing-content"
          value={proposal.instructions}
          onChange={(e) => set("instructions", e.target.value)}
        />
      </Field>
      <Part title="When it runs">
        {proposal.schedule === null && (
          <p className="text-sm text-muted-foreground">Only when you talk to it</p>
        )}
        <ScheduleEditor
          schedule={proposal.schedule}
          stored={null}
          onChange={(next) => set("schedule", next)}
          error={scheduleProblem}
          preview={{
            times: sendable && preview.data ? preview.data : [],
            pending: preview.isFetching,
          }}
        />
      </Part>
      <Part title="Where it works">
        <p className="text-sm break-all">
          {proposal.folder ?? `A new folder: ${props.newDotRoot}\\${folderName(proposal.name)}`}
        </p>
        <div className="flex gap-2">
          <Button type="button" variant="outline" size="sm" onClick={chooseFolder}>
            Change
          </Button>
          {proposal.folder !== null && (
            <Button type="button" variant="outline" size="sm" onClick={() => set("folder", null)}>
              Use a new folder
            </Button>
          )}
        </div>
      </Part>
      <Part title="What it may do">
        {preset && <p className="text-sm">{preset[2]}</p>}
        <NativeSelect
          aria-label="Permissions"
          value={proposal.preset}
          onChange={(e) => set("preset", Preset.parse(e.target.value))}
        >
          {PRESETS.map(([p, label]) => (
            <option key={p} value={p}>
              {label}
            </option>
          ))}
        </NativeSelect>
        {proposal.extras.length > 0 && (
          <ul className="flex flex-col gap-1">
            {proposal.extras.map((extra) => {
              const line = extraLine(extra);
              return (
                <li key={extra} className="flex items-center gap-2 text-sm">
                  <span>{line}</span>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon-xs"
                    aria-label={`Remove ${line}`}
                    title="Remove"
                    onClick={() =>
                      set(
                        "extras",
                        proposal.extras.filter((e) => e !== extra),
                      )
                    }
                  >
                    <X aria-hidden />
                  </Button>
                </li>
              );
            })}
          </ul>
        )}
      </Part>
      {used.length > 0 && (
        <Part title="Uses">
          <ul aria-label="Uses" className="flex flex-wrap gap-1.5">
            {used.map(([kind, item]) => (
              <li
                key={`${kind}/${item}`}
                className="flex items-center gap-1 rounded-full border py-0.5 pr-1 pl-2.5 text-xs"
              >
                <span>{item}</span>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-xs"
                  aria-label={`Remove ${item}`}
                  title="Remove"
                  onClick={() => removeUse(kind, item)}
                >
                  <X aria-hidden />
                </Button>
              </li>
            ))}
          </ul>
        </Part>
      )}
    </div>
  );
}
