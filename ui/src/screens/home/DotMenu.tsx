import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Check, Settings } from "lucide-react";
import { DropdownMenu } from "radix-ui";
import { useNavigate } from "react-router";

import { commands } from "@/api/commands";
import type { Dot } from "@/api/types";
import { Button } from "@/components/ui/button";
import { notifyError } from "@/lib/notify";
import { invalidate, invalidateDots, keys, setDotEnabledMutation, useSwitchingDot } from "@/lib/query";

const ITEM =
  "flex cursor-default items-center gap-2 rounded-sm px-2 py-1.5 outline-none select-none data-[disabled]:text-muted-foreground data-[highlighted]:bg-accent";

/// The header's ⚙: what the Dots list offered for a dot. Edit opens its editor, Memory… its
/// editor's Memory tab, Run now starts a run (it shows up in the conversation as a card),
/// Enabled switches it on or off.
export function DotMenu({ dot }: { dot: Dot }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const switching = useSwitchingDot(dot.id);
  const runNow = useMutation({
    mutationFn: () => commands.runNow(dot.id),
    onSuccess: () => invalidate(qc, ["runs"]),
    onError: notifyError,
  });
  const setEnabled = useMutation({
    ...setDotEnabledMutation,
    // The editor fills its form from the cached dot when it opens, so that copy must never be
    // older than this change. It is dropped before the command goes out, and the command's
    // answer is the new copy; without an answer the editor loads the dot itself. An editor
    // opened while the command is on its way waits for it (`useSwitchingDot`).
    onMutate: (v) => qc.removeQueries({ queryKey: keys.dot(v.id) }),
    onSuccess: (saved) => void qc.setQueryData(keys.dot(saved.id), saved),
    onSettled: () => invalidateDots(qc),
    onError: notifyError,
  });
  return (
    <DropdownMenu.Root modal={false}>
      <DropdownMenu.Trigger asChild>
        <Button size="icon-sm" variant="ghost" aria-label="Dot settings" title="Dot settings">
          <Settings aria-hidden />
        </Button>
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content
          align="end"
          className="z-50 w-48 rounded-md border bg-popover p-1 text-sm text-popover-foreground shadow-md"
        >
          <DropdownMenu.Item className={ITEM} onSelect={() => void navigate(`/dots/${dot.id}`)}>
            Edit
          </DropdownMenu.Item>
          <DropdownMenu.Item
            className={ITEM}
            onSelect={() => void navigate(`/dots/${dot.id}?tab=memory`)}
          >
            Memory…
          </DropdownMenu.Item>
          <DropdownMenu.Item
            className={ITEM}
            disabled={runNow.isPending}
            onSelect={() => runNow.mutate()}
          >
            Run now
          </DropdownMenu.Item>
          <DropdownMenu.CheckboxItem
            className={ITEM}
            checked={dot.enabled}
            disabled={switching}
            onCheckedChange={(enabled) => setEnabled.mutate({ id: dot.id, enabled })}
          >
            <span className="flex size-4 items-center justify-center">
              <DropdownMenu.ItemIndicator>
                <Check aria-hidden className="size-4" />
              </DropdownMenu.ItemIndicator>
            </span>
            Enabled
          </DropdownMenu.CheckboxItem>
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}
