import { Clock } from "lucide-react";
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
import { fromPicker, isLater, laterPresets, toPicker, type LaterPreset } from "@/lib/later";

const ITEM =
  "flex cursor-default items-center gap-2 rounded-sm px-2 py-1.5 outline-none select-none data-[highlighted]:bg-accent";

/// A menu of later times (snooze and done spec section 6): the presets as of when it opens,
/// and "Pick a date and time…", which asks for one in a dialog. `label` names the button, the
/// dialog (`<label> until`) and its confirm button. Send later reuses it.
export function LaterMenu(props: {
  label: string;
  onPick: (at: Date) => void;
  disabled?: boolean;
}) {
  const [presets, setPresets] = useState<LaterPreset[]>([]);
  const [picking, setPicking] = useState(false);
  return (
    <>
      <DropdownMenu.Root
        modal={false}
        // The presets are worked out when the menu opens, so "This evening" is not offered
        // once it is past 6 PM, however long the screen was open.
        onOpenChange={(open) => open && setPresets(laterPresets(new Date()))}
      >
        <DropdownMenu.Trigger asChild>
          <Button size="xs" variant="ghost" disabled={props.disabled}>
            <Clock aria-hidden />
            {props.label}
          </Button>
        </DropdownMenu.Trigger>
        <DropdownMenu.Portal>
          <DropdownMenu.Content
            align="end"
            className="z-50 w-56 rounded-md border bg-popover p-1 text-sm text-popover-foreground shadow-md"
          >
            {presets.map((preset) => (
              <DropdownMenu.Item
                key={preset.id}
                className={ITEM}
                onSelect={() => props.onPick(preset.at)}
              >
                {preset.label}
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
          label={props.label}
          onPick={(at) => {
            setPicking(false);
            props.onPick(at);
          }}
          onClose={() => setPicking(false)}
        />
      )}
    </>
  );
}

/// Asks for a date and time; one not in the future (or more than a year ahead) is refused.
function PickDialog(props: { label: string; onPick: (at: Date) => void; onClose: () => void }) {
  const id = useId();
  // Starts an hour ahead, to the minute: a time that is taken as it is.
  const [value, setValue] = useState(() => toPicker(new Date(Date.now() + 3_600_000)));
  const at = fromPicker(value);
  const ok = at !== null && isLater(at, new Date());
  return (
    <Dialog open onOpenChange={(open) => !open && props.onClose()}>
      <DialogContent aria-describedby={undefined}>
        <DialogHeader>
          <DialogTitle>{props.label} until</DialogTitle>
        </DialogHeader>
        <form
          className="flex flex-col gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (ok && at) props.onPick(at);
          }}
        >
          <Label htmlFor={id}>Date and time</Label>
          <Input
            id={id}
            type="datetime-local"
            value={value}
            onChange={(e) => setValue(e.target.value)}
          />
          {value !== "" && !ok && (
            <p role="alert" className="text-xs text-destructive">
              Pick a time in the future.
            </p>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={props.onClose}>
              Cancel
            </Button>
            <Button type="submit" disabled={!ok}>
              {props.label}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
