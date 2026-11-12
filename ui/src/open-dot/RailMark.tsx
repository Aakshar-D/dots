import { useQuery } from "@tanstack/react-query";
import { cn } from "cn";
import { useNavigate } from "react-router";

import { commands } from "@/api/commands";
import { notifyError } from "@/lib/notify";
import { keys } from "@/lib/query";
import { useVoiceLive, useVoiceStatus } from "@/lib/use-voice";

import { Breath } from "./Breath";
import { talk, type StageCommands } from "./use-stage";

/// A hotkey as the settings store it ("CmdOrCtrl+Shift+Space"), as a person reads it here.
export function shortcutLabel(hotkey: string): string {
  const mac = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform);
  return hotkey
    .split("+")
    .map((part) => (/^(CmdOrCtrl|CommandOrControl)$/i.test(part) ? (mac ? "Cmd" : "Ctrl") : part))
    .join("+");
}

/// A hotkey as the settings store it, in `aria-keyshortcuts` form ("Control+Shift+Space").
export function ariaShortcut(hotkey: string): string {
  const mac = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform);
  const names: Record<string, string> = {
    cmdorctrl: mac ? "Meta" : "Control",
    commandorcontrol: mac ? "Meta" : "Control",
    ctrl: "Control",
    control: "Control",
    cmd: "Meta",
    command: "Meta",
    super: "Meta",
    meta: "Meta",
    option: "Alt",
    alt: "Alt",
    shift: "Shift",
  };
  return hotkey
    .split("+")
    .map((part) => names[part.toLowerCase()] ?? part)
    .join("+");
}

/// The Open Dot shortcut as set in Settings; "" when none is set or not yet read.
export function useVoiceHotkey(): string {
  return (
    useQuery({ queryKey: keys.settings, queryFn: commands.getSettings }).data?.voice_hotkey ?? ""
  ).trim();
}

/// The mark at the bottom of the rail: a small Breath that starts Open Dot. When voice is off,
/// not installed or still getting ready it opens Settings > Voice instead of a stage that
/// cannot listen. While Dot thinks or speaks it interrupts and listens, as the shortcut does;
/// otherwise it starts listening (a question that waits stays). Until the voice status is
/// known, a click does nothing.
export function RailMark({ act = commands }: { act?: StageCommands } = {}) {
  const navigate = useNavigate();
  const status = useVoiceStatus().data;
  const hotkey = useVoiceHotkey();
  const micOpen = useVoiceLive((live) => live.micOpen);
  const state = useVoiceLive((live) => live.state);
  const setUp = status !== undefined && !(status.enabled && status.ready);

  const label = setUp ? "Set up voice" : "Talk to Dot";
  const tip = hotkey && !setUp ? `${label} (${shortcutLabel(hotkey)})` : label;
  const click = () => {
    if (status === undefined) return;
    if (setUp) {
      void navigate("/settings/voice");
      return;
    }
    talk(act, state).catch(notifyError);
  };
  return (
    <button
      type="button"
      aria-label={label}
      aria-keyshortcuts={hotkey && !setUp ? ariaShortcut(hotkey) : undefined}
      title={micOpen ? "Dot is listening" : tip}
      data-listening={micOpen || undefined}
      onClick={click}
      className={cn(
        "flex size-9 shrink-0 items-center justify-center rounded-lg hover:bg-sidebar-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
        micOpen && "ring-1 ring-primary",
      )}
    >
      <Breath size={24} state="resting" quiet />
    </button>
  );
}
