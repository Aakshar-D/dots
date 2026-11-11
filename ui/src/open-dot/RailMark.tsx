import { useQuery } from "@tanstack/react-query";
import { cn } from "cn";
import { useNavigate } from "react-router";

import { commands } from "@/api/commands";
import { notifyError } from "@/lib/notify";
import { keys } from "@/lib/query";
import { useVoiceLive, useVoiceStatus } from "@/lib/use-voice";

import { Breath } from "./Breath";
import { useOpenDotBusy, type StageCommands } from "./use-stage";

/// A hotkey as the settings store it ("CmdOrCtrl+Shift+Space"), as a person reads it here.
export function shortcutLabel(hotkey: string): string {
  const mac = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform);
  return hotkey
    .split("+")
    .map((part) => (/^(CmdOrCtrl|CommandOrControl)$/i.test(part) ? (mac ? "Cmd" : "Ctrl") : part))
    .join("+");
}

/// The mark at the bottom of the rail: a small Breath that starts Open Dot. When voice is off,
/// not installed or still getting ready it opens Settings > Voice instead of a stage that
/// cannot listen. While Open Dot is busy it stops and listens, as the shortcut does.
export function RailMark({ act = commands }: { act?: StageCommands } = {}) {
  const navigate = useNavigate();
  const status = useVoiceStatus().data;
  const hotkey = useQuery({ queryKey: keys.settings, queryFn: commands.getSettings }).data
    ?.voice_hotkey;
  const micOpen = useVoiceLive((live) => live.micOpen);
  const busy = useOpenDotBusy();
  const ready = status?.enabled === true && status.ready;

  const label = hotkey ? `Talk to Dot (${shortcutLabel(hotkey)})` : "Talk to Dot";
  const click = () => {
    if (!ready) {
      void navigate("/settings/voice");
      return;
    }
    (busy ? act.voiceStop() : act.voiceOpen()).catch(notifyError);
  };
  return (
    <button
      type="button"
      aria-label={label}
      title={micOpen ? "Dot is listening" : label}
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
