import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Play } from "lucide-react";
import { useLayoutEffect, useRef, type ReactNode } from "react";
import { toast } from "sonner";

import { commands } from "@/api/commands";
import type { Settings, VoiceDevice, VoiceStatus } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { Switch } from "@/components/ui/switch";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { takeVoiceStatus, useVoiceLive, useVoiceStatus } from "@/lib/use-voice";

function Section(props: { title: string; children: ReactNode }) {
  return (
    <fieldset className="flex max-w-3xl flex-col gap-3 rounded-lg border p-4">
      <legend className="px-1 text-sm font-semibold">{props.title}</legend>
      {props.children}
    </fieldset>
  );
}

/// The ten voices, numbered from 1 for people and from 0 for the core.
const VOICES = Array.from({ length: 10 }, (_, i) => i);

/// The models Open Dot can think with: the ids the Claude model pickers list.
const MODELS = [
  { id: "haiku", label: "Fast" },
  { id: "sonnet", label: "Balanced" },
  { id: "opus", label: "Most capable" },
] as const;

const MIN_SPEED = 0.8;
const MAX_SPEED = 1.4;

/// A speed as the slider gives it, without float noise: 1.15, not 1.1500000000000001.
const roundSpeed = (speed: number) => Math.round(speed * 100) / 100;
/// "1.0×", "1.15×".
const speedLabel = (speed: number) =>
  `${Math.round(speed * 100) % 10 === 0 ? speed.toFixed(1) : speed.toFixed(2)}×`;

/// Bytes in whole megabytes, as the download is described ("about 260 MB").
const mb = (bytes: number) => `${Math.round(bytes / 1_000_000)} MB`;

/// What the status line says. `trouble` is the newest trouble: one that came as an event, or
/// the status's own.
function statusText(
  status: VoiceStatus | undefined,
  enabling: boolean,
  trouble: string | null,
): string {
  if (!status) return "";
  if (enabling || status.downloading) return "Getting ready";
  if (!status.enabled) return "Off";
  if (trouble) return trouble;
  return status.ready ? "Ready" : "Getting ready";
}

/// A select of the devices voice can use, "System default" first. A saved device that is not
/// plugged in now still shows, so the choice is not lost by looking at it.
function DeviceSelect(props: {
  id: string;
  label: string;
  /// Undefined until the list has loaded.
  devices: VoiceDevice[] | undefined;
  value: string | null;
  disabled: boolean;
  onChange: (id: string | null) => void;
}) {
  const known = props.devices?.some((d) => d.id === props.value) ?? false;
  const saved = props.value !== null && !known;
  return (
    <Field label={props.label} htmlFor={props.id}>
      <NativeSelect
        id={props.id}
        className="w-64"
        disabled={props.disabled}
        value={props.value ?? ""}
        onChange={(e) => props.onChange(e.target.value === "" ? null : e.target.value)}
      >
        <option value="">System default</option>
        {(props.devices ?? []).map((d) => (
          <option key={d.id} value={d.id}>
            {d.name}
          </option>
        ))}
        {saved && (
          <option value={props.value!}>
            {props.devices ? "Saved device (not connected)" : "Saved device"}
          </option>
        )}
      </NativeSelect>
    </Field>
  );
}

/// The download of the speech files: how far, and a way to stop it.
function Download(props: { totalBytes: number; onGone: () => void }) {
  const download = useVoiceLive((live) => live.download);
  const cancel = useMutation({ mutationFn: commands.voiceDownloadCancel, onError: notifyError });
  const cancelButton = useRef<HTMLButtonElement>(null);
  // Cancel goes away with the download: focus on it moves on instead of being lost.
  const gone = useRef(props.onGone);
  gone.current = props.onGone;
  useLayoutEffect(
    () => () => {
      if (cancelButton.current && document.activeElement === cancelButton.current) gone.current();
    },
    [],
  );
  const done = download?.done ?? 0;
  const total = download?.total || props.totalBytes;
  // Until the first count arrives there is nothing to measure.
  const known = download !== null;
  return (
    <div className="flex flex-col gap-1.5">
      <div
        role="progressbar"
        aria-label="Speech files downloaded"
        {...(known
          ? {
              "aria-valuemin": 0,
              "aria-valuemax": total,
              "aria-valuenow": done,
              "aria-valuetext": `${mb(done)} of ${mb(total)}`,
            }
          : { "aria-valuetext": "Getting the speech files" })}
        className="h-1 w-full max-w-sm overflow-hidden rounded-full bg-muted"
      >
        <div
          className={
            known
              ? "h-full rounded-full bg-primary transition-[width] motion-reduce:transition-none"
              : "h-full w-1/3 animate-pulse rounded-full bg-primary motion-reduce:animate-none"
          }
          style={
            known ? { width: `${total === 0 ? 0 : Math.min(100, (done / total) * 100)}%` } : undefined
          }
        />
      </div>
      <div className="flex items-center gap-3">
        <p className="text-xs text-muted-foreground">
          {known ? `${mb(done)} of ${mb(total)}` : "Getting the speech files…"}
        </p>
        <Button
          type="button"
          size="sm"
          variant="outline"
          aria-label="Cancel download"
          ref={cancelButton}
          aria-disabled={cancel.isPending}
          onClick={() => {
            if (!cancel.isPending) cancel.mutate();
          }}
        >
          Cancel
        </Button>
      </div>
    </div>
  );
}

/// Settings > Voice: Open Dot, the voice assistant. The switch, the download, previews and
/// removing the files act at once; the rest is part of the Settings form (`draft`, `set`).
/// `voice_enabled` is the core's to save (`voice_enable`), so the form never sends its own.
export function VoiceSection({
  draft,
  set,
}: {
  draft: Settings;
  set: (change: Partial<Settings>) => void;
}) {
  const qc = useQueryClient();
  const status = useVoiceStatus();
  const s = status.data;
  const ready = s?.ready === true;
  const devices = useQuery({
    queryKey: keys.voiceDevices,
    queryFn: commands.voiceDevices,
    // Listing devices opens none, so it does not wait for the session.
    enabled: s?.enabled === true,
  });
  const trouble = useVoiceLive((live) => live.trouble) ?? s?.trouble ?? null;
  const toggle = useRef<HTMLButtonElement>(null);

  /// The core saved `voice_enabled` itself: the stored settings are read again.
  const took = (next: VoiceStatus) => {
    takeVoiceStatus(qc, next);
    invalidate(qc, keys.settings);
  };
  const enable = useMutation({
    mutationFn: commands.voiceEnable,
    onSuccess: took,
    onError: (e) => {
      notifyError(e);
      invalidate(qc, keys.voice);
      invalidate(qc, keys.settings);
    },
  });
  const remove = useMutation({
    mutationFn: commands.voiceRemove,
    onSuccess: (next) => {
      took(next);
      toast.success("Speech files removed");
    },
    onError: notifyError,
  });
  const preview = useMutation({
    mutationFn: (voice: number) => commands.voicePreview(voice, draft.voice_speed),
    onError: notifyError,
  });

  const enabling = enable.isPending && enable.variables === true;
  const downloading = enabling || s?.downloading === true;
  // A download that began elsewhere (before the tab, or before the app restarted) counts.
  const on = enabling || downloading || (s?.enabled ?? false);
  const busy = !s || enable.isPending || remove.isPending || downloading;
  const needsSession = !s?.enabled
    ? "Turn Open Dot on to try the voices and choose devices."
    : !ready
      ? "Open Dot is still getting ready."
      : null;

  return (
    <>
      <Section title="Open Dot">
        <div className="flex items-center gap-2">
          <Switch
            id="settings-voice"
            ref={toggle}
            checked={on}
            // Not disabled: a disabled switch drops focus, and a click only needs ignoring.
            aria-disabled={busy}
            onCheckedChange={(next) => {
              if (!busy) enable.mutate(next);
            }}
          />
          <Label htmlFor="settings-voice">Open Dot</Label>
        </div>
        <p className="text-xs text-muted-foreground">
          Open Dot lets you talk to Dots and hear it answer.
        </p>
        {s && !s.installed && !downloading && (
          <p className="text-xs text-muted-foreground">
            Turning it on downloads about 260 MB of speech files once; they stay on this computer.
          </p>
        )}
        <p role="status" aria-label="Open Dot status" className="flex gap-1.5 text-xs">
          <span className={s?.enabled && trouble && !downloading ? "text-destructive" : undefined}>
            {statusText(s, enabling, trouble)}
          </span>
          {s?.mic_open && (
            <>
              <span aria-hidden className="text-muted-foreground">
                ·
              </span>
              <span>Listening</span>
            </>
          )}
        </p>
        {downloading && <Download totalBytes={s?.download_bytes ?? 0} onGone={() => toggle.current?.focus()} />}
      </Section>

      <Section title="Voice">
        <div role="radiogroup" aria-label="Voice" className="grid grid-cols-2 gap-2 sm:grid-cols-3 xl:grid-cols-5">
          {VOICES.map((i) => (
            <div key={i} className="flex items-center gap-1.5 rounded-md border px-2 py-1">
              <input
                type="radio"
                id={`settings-voice-${i}`}
                name="settings-voice-id"
                className="accent-primary"
                checked={draft.voice_id === i}
                onChange={() => set({ voice_id: i })}
              />
              <Label htmlFor={`settings-voice-${i}`} className="flex-1 font-normal whitespace-nowrap">
                Voice {i + 1}
              </Label>
              <Button
                type="button"
                size="icon-sm"
                variant="ghost"
                aria-label={`Play voice ${i + 1}`}
                disabled={!ready || preview.isPending}
                onClick={() => preview.mutate(i)}
              >
                <Play aria-hidden />
              </Button>
            </div>
          ))}
        </div>
        <Field label="Speaking speed" htmlFor="settings-voice-speed">
          <div className="flex items-center gap-3">
            <input
              id="settings-voice-speed"
              type="range"
              min={MIN_SPEED}
              max={MAX_SPEED}
              step={0.05}
              className="w-64 accent-primary"
              aria-valuetext={speedLabel(draft.voice_speed)}
              value={draft.voice_speed}
              onChange={(e) => set({ voice_speed: roundSpeed(Number(e.target.value)) })}
            />
            <span className="w-12 text-sm tabular-nums">{speedLabel(draft.voice_speed)}</span>
          </div>
        </Field>
        {needsSession && <p className="text-xs text-muted-foreground">{needsSession}</p>}
        <div className="flex items-center gap-2">
          <Switch
            id="settings-voice-notices"
            checked={draft.voice_notices}
            onCheckedChange={(voice_notices) => set({ voice_notices })}
          />
          <Label htmlFor="settings-voice-notices">Tell me aloud when something needs me</Label>
        </div>
      </Section>

      <Section title="Listening">
        <Field
          label="Hotkey"
          htmlFor="settings-voice-hotkey"
          hint="Use names like Ctrl, Shift, Alt and a key, joined with +."
        >
          <div className="flex gap-2">
            <Input
              id="settings-voice-hotkey"
              className="w-64 font-mono"
              placeholder="No hotkey"
              aria-describedby={s?.hotkey_error ? "settings-voice-hotkey-error" : undefined}
              value={draft.voice_hotkey}
              onChange={(e) => set({ voice_hotkey: e.target.value })}
            />
            <Button
              type="button"
              variant="outline"
              aria-label="Clear the hotkey"
              disabled={draft.voice_hotkey === ""}
              onClick={() => set({ voice_hotkey: "" })}
            >
              Clear
            </Button>
          </div>
        </Field>
        {s?.hotkey_error && (
          <p id="settings-voice-hotkey-error" role="alert" className="text-xs text-destructive">
            {s.hotkey_error}
          </p>
        )}
        <div className="flex items-center gap-2">
          <Switch
            id="settings-voice-wake"
            checked={draft.voice_wake_phrase}
            onCheckedChange={(voice_wake_phrase) => set({ voice_wake_phrase })}
          />
          <Label htmlFor="settings-voice-wake">Hey Dot</Label>
        </div>
        <p className="text-xs text-muted-foreground">
          The microphone stays on while Dots is running so it can hear the phrase. Audio never
          leaves this computer.
        </p>
        <div className="flex items-center gap-2">
          <Switch
            id="settings-voice-interrupt"
            checked={draft.voice_interrupt_by_talking}
            onCheckedChange={(voice_interrupt_by_talking) => set({ voice_interrupt_by_talking })}
          />
          <Label htmlFor="settings-voice-interrupt">Interrupt by just talking</Label>
        </div>
        <p className="text-xs text-muted-foreground">Best with headphones.</p>
        <div className="flex items-center gap-2">
          <Switch
            id="settings-voice-tone"
            checked={draft.voice_tone}
            onCheckedChange={(voice_tone) => set({ voice_tone })}
          />
          <Label htmlFor="settings-voice-tone">Play a tone when Dot starts listening</Label>
        </div>
      </Section>

      <Section title="Devices">
        <div className="flex flex-wrap gap-4">
          <DeviceSelect
            id="settings-voice-mic"
            label="Microphone"
            devices={devices.data?.microphones}
            value={draft.voice_mic}
            disabled={!ready}
            onChange={(voice_mic) => set({ voice_mic })}
          />
          <DeviceSelect
            id="settings-voice-speaker"
            label="Speaker"
            devices={devices.data?.speakers}
            value={draft.voice_speaker}
            disabled={!ready}
            onChange={(voice_speaker) => set({ voice_speaker })}
          />
        </div>
        {needsSession && <p className="text-xs text-muted-foreground">{needsSession}</p>}
      </Section>

      <Section title="Model">
        <Field label="Which model thinks" htmlFor="settings-open-dot-model">
          <NativeSelect
            id="settings-open-dot-model"
            className="w-64"
            value={draft.open_dot_model}
            onChange={(e) => set({ open_dot_model: e.target.value })}
          >
            {MODELS.map((m) => (
              <option key={m.id} value={m.id}>
                {m.label}
              </option>
            ))}
            {!MODELS.some((m) => m.id === draft.open_dot_model) && (
              <option value={draft.open_dot_model}>{draft.open_dot_model}</option>
            )}
          </NativeSelect>
        </Field>
      </Section>

      {s?.installed && (
        <Section title="Speech files">
          <p className="text-xs text-muted-foreground">
            The speech files take about {mb(s.download_bytes)} on this computer.
          </p>
          <div>
            <ConfirmButton
              title="Remove the speech files?"
              description="Open Dot turns off. Turning it on again downloads them again."
              confirmLabel="Remove"
              disabled={remove.isPending || enable.isPending}
              onConfirm={() => remove.mutate()}
            >
              Remove speech files
            </ConfirmButton>
          </div>
        </Section>
      )}
    </>
  );
}
