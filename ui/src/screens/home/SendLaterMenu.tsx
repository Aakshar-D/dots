import { ChevronUp } from "lucide-react";
import { DropdownMenu } from "radix-ui";
import { useId, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { localWeekdayTime } from "@/lib/format";
import { fromPicker, laterPresets, toPicker, type LaterPreset } from "@/lib/later";
import { errorMessage, notifyError } from "@/lib/notify";

const ITEM =
  "flex cursor-default items-center gap-2 rounded-sm px-2 py-1.5 outline-none select-none data-[highlighted]:bg-accent";

/// The presets of `@/lib/later` with Send later's names: the Monday preset says when it is.
function presets(now: Date): LaterPreset[] {
  return laterPresets(now).map((p) => (p.id === "next_week" ? { ...p, label: "Monday 9 AM" } : p));
}

/// The arrow beside Send (send later spec section 6): a menu of the later presets, each with
/// its time, and "Pick a date and time…", which asks for one in a dialog. `onPick` schedules
/// the message; a refusal shows in the dialog, or as the usual error notice from the menu.
export function SendLaterMenu(props: {
  disabled: boolean;
  onPick: (at: Date) => Promise<unknown>;
}) {
  const [shown, setShown] = useState<LaterPreset[]>([]);
  const [picking, setPicking] = useState(false);
  return (
    <>
      <DropdownMenu.Root
        modal={false}
        // Worked out when the menu opens, so a preset that has passed is not offered.
        onOpenChange={(open) => open && setShown(presets(new Date()))}
      >
        <DropdownMenu.Trigger asChild>
          <Button
            size="icon-sm"
            variant="ghost"
            className="shrink-0 rounded-full"
            aria-label="Send later"
            title="Send later"
            disabled={props.disabled}
          >
            <ChevronUp aria-hidden />
          </Button>
        </DropdownMenu.Trigger>
        <DropdownMenu.Portal>
          <DropdownMenu.Content
            align="end"
            className="z-50 w-72 rounded-md border bg-popover p-1 text-sm text-popover-foreground shadow-md"
          >
            <DropdownMenu.Label className="px-2 py-1.5 text-xs text-muted-foreground">
              Send later
            </DropdownMenu.Label>
            {shown.map((preset) => (
              <DropdownMenu.Item
                key={preset.id}
                className={ITEM}
                onSelect={() => void props.onPick(preset.at).catch(notifyError)}
              >
                <span className="flex-1">{preset.label}</span>
                <span className="text-xs text-muted-foreground">
                  {localWeekdayTime(preset.at.toISOString())}
                </span>
              </DropdownMenu.Item>
            ))}
            <DropdownMenu.Separator className="my-1 h-px bg-border" />
            <DropdownMenu.Item className={ITEM} onSelect={() => setPicking(true)}>
              Pick a date and time…
            </DropdownMenu.Item>
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu.Root>
      {picking && (
        <PickDialog
          onPick={(at) => props.onPick(at).then(() => setPicking(false))}
          onClose={() => setPicking(false)}
        />
      )}
    </>
  );
}

/// Asks for a date and time; the core's refusal of it shows under the field.
function PickDialog(props: { onPick: (at: Date) => Promise<unknown>; onClose: () => void }) {
  const id = useId();
  // Starts an hour ahead, to the minute.
  const [value, setValue] = useState(() => toPicker(new Date(Date.now() + 3_600_000)));
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const at = fromPicker(value);
  return (
    <Dialog open onOpenChange={(open) => !open && props.onClose()}>
      <DialogContent aria-describedby={undefined}>
        <DialogHeader>
          <DialogTitle>Send later</DialogTitle>
        </DialogHeader>
        <form
          className="flex flex-col gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (at === null || pending) return;
            setError(null);
            setPending(true);
            props.onPick(at).then(
              () => setPending(false),
              (err: unknown) => {
                setPending(false);
                setError(errorMessage(err));
              },
            );
          }}
        >
          <Label htmlFor={id}>Date and time</Label>
          <Input
            id={id}
            type="datetime-local"
            value={value}
            onChange={(e) => setValue(e.target.value)}
          />
          {error !== null && (
            <p role="alert" className="text-xs text-destructive">
              {error}
            </p>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={props.onClose}>
              Cancel
            </Button>
            <Button type="submit" disabled={at === null || pending}>
              Schedule
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
