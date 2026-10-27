import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Plus, Trash2 } from "lucide-react";
import { useState, type ReactNode } from "react";
import { toast } from "sonner";

import { CommandError, commands } from "@/api/commands";
import { LIMITS, type Settings, type Status } from "@/api/types";
import { AppearanceSetting } from "@/components/AppearanceSetting";
import { BrowserSettings } from "@/components/BrowserSettings";
import { ErrorNote } from "@/components/ErrorNote";
import { Field } from "@/components/Field";
import { Page } from "@/components/Page";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys, takeStatus } from "@/lib/query";

import { ChatGptAccount } from "./ChatGptAccount";
import { Organisations } from "./Organisations";

type Errors = { port?: string; max?: string; global?: string; share?: string };

/// The core's words for a hold share out of bounds (usage and limits spec section 4).
const SHARE_RULE = "A share is 10 to 100%, in steps of 5.";

/// The size of `text` in UTF-8 bytes, as the core counts it.
export const utf8Bytes = (text: string) => new TextEncoder().encode(text).length;

/// What is wrong with the numbers and the global instructions, mirroring `Settings::validate`
/// in the core.
export function settingsErrors(
  draft: Pick<Settings, "port" | "max_concurrent_runs"> &
    Partial<Pick<Settings, "global_instructions" | "hold_share_five_hour" | "hold_share_weekly">>,
): Errors {
  const errors: Errors = {};
  if (utf8Bytes(draft.global_instructions ?? "") > LIMITS.max_global_instructions) {
    errors.global = "The global instructions are at most 32 KB.";
  }
  const whole = (n: number, min: number, max: number) => Number.isInteger(n) && n >= min && n <= max;
  if (!whole(draft.port, 1, 65535)) errors.port = "The port must be a whole number, 1 to 65535.";
  if (!whole(draft.max_concurrent_runs, 1, LIMITS.max_concurrent_runs)) {
    errors.max = `Runs at once must be a whole number, 1 to ${LIMITS.max_concurrent_runs}.`;
  }
  const share = (n: number | undefined) => n === undefined || (whole(n, 10, 100) && n % 5 === 0);
  if (!share(draft.hold_share_five_hour) || !share(draft.hold_share_weekly)) {
    errors.share = SHARE_RULE;
  }
  return errors;
}

function Section(props: { title: string; children: ReactNode }) {
  return (
    <fieldset className="flex max-w-3xl flex-col gap-3 rounded-lg border p-4">
      <legend className="px-1 text-sm font-semibold">{props.title}</legend>
      {props.children}
    </fieldset>
  );
}

const numberOf = (text: string) => (text === "" ? Number.NaN : Number(text));
const shown = (n: number) => (Number.isNaN(n) ? "" : n);

/// One local endpoint: remove it, or ask it for a tool call to see that it works.
function Endpoint(props: { url: string; onRemove: () => void }) {
  const [model, setModel] = useState("");
  const test = useMutation({
    mutationFn: () => commands.testEndpoint(props.url, model.trim()),
  });
  return (
    <li aria-label={props.url} className="flex flex-col gap-1.5 rounded-md border p-2">
      <div className="flex items-center gap-2">
        <span className="min-w-0 flex-1 truncate font-mono text-xs">{props.url}</span>
        <Input
          aria-label={`Model for ${props.url}`}
          placeholder="Model to test with"
          className="w-56"
          value={model}
          onChange={(e) => setModel(e.target.value)}
        />
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={test.isPending || model.trim() === ""}
          onClick={() => test.mutate()}
        >
          {test.isPending ? "Testing…" : "Test endpoint"}
        </Button>
        <Button
          type="button"
          size="icon-sm"
          variant="ghost"
          aria-label={`Remove ${props.url}`}
          onClick={props.onRemove}
        >
          <Trash2 aria-hidden />
        </Button>
      </div>
      {test.isSuccess && (
        <p role="status" className="text-xs text-success">
          {test.data}
        </p>
      )}
      {test.isError && (
        <p role="alert" className="text-xs text-destructive">
          {errorMessage(test.error)}
        </p>
      )}
    </li>
  );
}

function Form({ stored, status }: { stored: Settings; status: Status | undefined }) {
  const qc = useQueryClient();
  const [draft, setDraft] = useState(stored);
  const [claudePath, setClaudePath] = useState(stored.claude_path ?? "");
  const [newEndpoint, setNewEndpoint] = useState("");
  const [endpointError, setEndpointError] = useState<string>();
  const [tried, setTried] = useState(false);
  const set = (change: Partial<Settings>) => setDraft({ ...draft, ...change });

  const save = useMutation({
    mutationFn: commands.updateSettings,
    onSuccess: (saved) => {
      qc.setQueryData(keys.settings, saved.settings);
      // The restart banner comes from the status.
      invalidate(qc, keys.status);
      toast.success(saved.restart_required ? "Saved. Restart Dots to apply." : "Saved");
    },
  });
  // Declining a native confirmation is the user's own answer, not a failure. The answer names
  // the program that was not changed.
  const cancelled = save.error instanceof CommandError && save.error.kind === "cancelled";
  const declined = cancelled && save.error?.message === "the Claude program was not changed";
  const edgeDeclined = cancelled && save.error?.message === "the Edge program was not changed";
  const pause = useMutation({
    mutationFn: commands.setSchedulesPaused,
    onSuccess: (next) => {
      takeStatus(qc, next);
      // A pause or a resume moves every schedule's next run, which the Dots list holds.
      invalidate(qc, keys.overview);
    },
    onError: notifyError,
  });
  const markTested = useMutation({
    mutationFn: commands.markClaudeTested,
    onSuccess: (next) => takeStatus(qc, next),
    onError: notifyError,
  });

  const errors = settingsErrors(draft);
  const shareBad = tried && errors.share !== undefined;
  function submit() {
    setTried(true);
    if (Object.keys(errors).length > 0) return;
    save.mutate({
      ...draft,
      claude_path: claudePath.trim() || null,
      edge_path: draft.edge_path?.trim() || null,
    });
  }
  function addEndpoint() {
    const url = newEndpoint.trim();
    if (!/^http:\/\/[^\s/]+/.test(url)) {
      setEndpointError("An endpoint starts with http:// and names a host.");
      return;
    }
    if (!draft.local_endpoints.includes(url)) {
      set({ local_endpoints: [...draft.local_endpoints, url] });
    }
    setNewEndpoint("");
    setEndpointError(undefined);
  }

  const claude = status?.claude;
  return (
    <Page
      title="Settings"
      actions={
        <Button size="sm" disabled={save.isPending} onClick={submit}>
          Save
        </Button>
      }
    >
      {save.isError && !cancelled && <ErrorNote error={save.error} />}
      <AppearanceSetting />

      <ChatGptAccount />

      <Organisations />

      <Section title="General">
        <div className="flex items-center gap-2">
          <Switch
            id="settings-autostart"
            checked={draft.autostart}
            onCheckedChange={(autostart) => set({ autostart })}
          />
          <Label htmlFor="settings-autostart">Start Dots when I sign in</Label>
        </div>
        <div className="flex items-center gap-2">
          <Switch
            id="settings-notify-notes"
            checked={draft.notify_on_notes}
            onCheckedChange={(notify_on_notes) => set({ notify_on_notes })}
          />
          <Label htmlFor="settings-notify-notes">Notify on dot notes</Label>
        </div>
        <p className="text-xs text-muted-foreground">Questions from dots always notify.</p>
        <div className="flex items-center gap-2">
          <Switch
            id="settings-paused"
            checked={status?.schedules_paused ?? false}
            disabled={!status || pause.isPending}
            onCheckedChange={(paused) => pause.mutate(paused)}
          />
          <Label htmlFor="settings-paused">Pause schedules and watchers</Label>
        </div>
        <p className="text-xs text-muted-foreground">
          Pausing applies at once and is not part of Save. Webhooks, "Run now" and chat still
          work while schedules and watchers are paused.
        </p>
        <Field label="Data folder" htmlFor="settings-data-dir">
          <Input
            id="settings-data-dir"
            readOnly
            className="font-mono"
            value={status?.data_dir ?? ""}
          />
        </Field>
      </Section>

      <Section title="Global instructions">
        <Field
          label="Global instructions"
          htmlFor="settings-global"
          error={tried ? errors.global : undefined}
          hint={`Every dot's first prompt starts with these. A dot can switch them off in its editor. ${utf8Bytes(draft.global_instructions)} of ${LIMITS.max_global_instructions} bytes.`}
        >
          <Textarea
            id="settings-global"
            rows={6}
            value={draft.global_instructions}
            onChange={(e) => set({ global_instructions: e.target.value })}
          />
        </Field>
      </Section>

      <Section title="Local server and runs">
        <Field
          label="Port"
          htmlFor="settings-port"
          error={tried ? errors.port : undefined}
          hint="Webhooks and the Claude CLI's permission prompts reach Dots on 127.0.0.1 at this port. Takes effect after a restart."
        >
          <Input
            id="settings-port"
            type="number"
            className="w-40"
            value={shown(draft.port)}
            onChange={(e) => set({ port: numberOf(e.target.value) })}
          />
        </Field>
        <Field
          label="Runs at once"
          htmlFor="settings-max"
          error={tried ? errors.max : undefined}
          hint="More runs wait in the queue. Takes effect after a restart."
        >
          <Input
            id="settings-max"
            type="number"
            className="w-40"
            value={shown(draft.max_concurrent_runs)}
            onChange={(e) => set({ max_concurrent_runs: numberOf(e.target.value) })}
          />
        </Field>
      </Section>

      <Section title="Claude usage">
        <div className="flex items-center gap-2">
          <Switch
            id="settings-hold"
            checked={draft.hold_enabled}
            onCheckedChange={(hold_enabled) => set({ hold_enabled })}
          />
          <Label htmlFor="settings-hold">Hold background dots when Claude usage is high</Label>
        </div>
        <p className="text-xs text-muted-foreground">
          Scheduled, watcher, follow-up and webhook runs wait while your Claude usage is at or
          above a share; chat and Run now never wait. Usage counts your own Claude use too.
        </p>
        <div className="flex flex-wrap gap-4">
          <Field label="5-hour window share (%)" htmlFor="settings-share-five">
            <Input
              id="settings-share-five"
              type="number"
              min={10}
              max={100}
              step={5}
              className="w-28"
              aria-invalid={shareBad || undefined}
              aria-describedby={shareBad ? "settings-share-error" : undefined}
              value={shown(draft.hold_share_five_hour)}
              onChange={(e) => set({ hold_share_five_hour: numberOf(e.target.value) })}
            />
          </Field>
          <Field label="Weekly window share (%)" htmlFor="settings-share-weekly">
            <Input
              id="settings-share-weekly"
              type="number"
              min={10}
              max={100}
              step={5}
              className="w-28"
              aria-invalid={shareBad || undefined}
              aria-describedby={shareBad ? "settings-share-error" : undefined}
              value={shown(draft.hold_share_weekly)}
              onChange={(e) => set({ hold_share_weekly: numberOf(e.target.value) })}
            />
          </Field>
        </div>
        {shareBad && (
          <p id="settings-share-error" role="alert" className="text-sm text-destructive">
            {errors.share}
          </p>
        )}
      </Section>

      <Section title="Claude Code CLI">
        <dl className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1 text-xs">
          <dt className="text-muted-foreground">In use</dt>
          <dd className="font-mono">{claude?.program ?? "not found"}</dd>
          <dt className="text-muted-foreground">Version</dt>
          <dd>{claude?.version ?? "unknown"}</dd>
          <dt className="text-muted-foreground">Tested with</dt>
          <dd>{claude?.tested_version ?? ""}</dd>
        </dl>
        {claude?.mismatch && (
          <div className="flex items-center gap-3">
            <p className="flex-1 text-xs text-needs">
              This version has not been tested with Dots. If your dots run as expected, mark it
              as tested.
            </p>
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={markTested.isPending}
              onClick={() => markTested.mutate()}
            >
              Mark as tested
            </Button>
          </div>
        )}
        <Field
          label="Program path"
          htmlFor="settings-claude-path"
          error={declined ? "Not saved: you declined the change of the Claude program." : undefined}
          hint="Leave empty to find claude on your PATH. A new program must be confirmed in a system dialog, and takes effect after a restart."
        >
          <Input
            id="settings-claude-path"
            className="font-mono"
            value={claudePath}
            onChange={(e) => {
              setClaudePath(e.target.value);
              // "Not saved" is about the path that was declined, not about this one.
              if (declined) save.reset();
            }}
          />
        </Field>
      </Section>

      <Section title="Browser">
        <BrowserSettings
          edgePath={draft.edge_path ?? ""}
          declined={edgeDeclined}
          onEdgePath={(edge_path) => {
            set({ edge_path });
            // "Not saved" is about the path that was declined, not about this one.
            if (edgeDeclined) save.reset();
          }}
        />
      </Section>

      <Section title="Local endpoints">
        <p className="text-xs text-muted-foreground">
          OpenAI-compatible servers that local dots can use, such as LM Studio or Ollama. Any
          http:// host is accepted, so a model server on your network works too. Requests carry
          your dots' instructions and file contents: list only servers you trust.
        </p>
        {draft.local_endpoints.length > 0 && (
          <ul className="flex flex-col gap-2">
            {draft.local_endpoints.map((url) => (
              <Endpoint
                key={url}
                url={url}
                onRemove={() =>
                  set({ local_endpoints: draft.local_endpoints.filter((u) => u !== url) })
                }
              />
            ))}
          </ul>
        )}
        <Field label="Add an endpoint" htmlFor="settings-new-endpoint" error={endpointError}>
          <div className="flex gap-2">
            <Input
              id="settings-new-endpoint"
              className="font-mono"
              placeholder="http://127.0.0.1:1234"
              value={newEndpoint}
              onChange={(e) => setNewEndpoint(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") addEndpoint();
              }}
            />
            <Button type="button" variant="outline" onClick={addEndpoint}>
              <Plus aria-hidden /> Add
            </Button>
          </div>
        </Field>
      </Section>
    </Page>
  );
}

export function SettingsScreen() {
  const settings = useQuery({ queryKey: keys.settings, queryFn: commands.getSettings });
  const status = useQuery({ queryKey: keys.status, queryFn: commands.status }).data;
  if (settings.isPending) return <Page title="Settings" />;
  if (settings.isError) {
    return (
      <Page title="Settings">
        <ErrorNote error={settings.error} />
      </Page>
    );
  }
  return <Form stored={settings.data} status={status} />;
}
