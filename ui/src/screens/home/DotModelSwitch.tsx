import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { toast } from "sonner";

import { commands } from "@/api/commands";
import type { Dot } from "@/api/types";
import { ModelPicker, useModels } from "@/components/ModelPicker";
import { useDotAccessQuery } from "@/lib/access";
import { specOf } from "@/lib/dot-rules";
import { notifyError } from "@/lib/notify";
import { invalidateDots, keys } from "@/lib/query";

/// The words beside the switch: the change is for the next reply.
export const NEXT_REPLY =
  "Model for the next reply. A reply already being written keeps the model it started with.";

/// The dot's model above its conversation (model switcher spec section 2.4). A change saves the
/// dot with `update_dot`, as the editor does, so every rule on editing a dot applies; it is
/// made to the dot as the core has it now, so nothing else an editor saved meanwhile is undone.
export function DotModelSwitch({ dot }: { dot: Dot }) {
  const qc = useQueryClient();
  const models = useModels(dot.engine, dot.endpoint_url);
  const access = useDotAccessQuery(dot.id);
  const refusal = access.data?.edit ?? undefined;
  // A refused change starts the picker afresh, so a Custom box it showed goes too.
  const [refused, setRefused] = useState(0);
  const save = useMutation({
    mutationFn: async (model: string) => {
      const now = await commands.getDot(dot.id);
      return commands.updateDot(dot.id, { ...specOf(now), model });
    },
    // The editor fills its form from the cached dot, so that copy must not be older than this.
    onMutate: () => qc.removeQueries({ queryKey: keys.dot(dot.id) }),
    onSuccess: (saved) => {
      qc.setQueryData(keys.dot(saved.id), saved);
      const label = models.options.find((m) => m.id === saved.model)?.label ?? saved.model;
      toast.success(`${saved.name} will use ${label} from its next reply.`);
    },
    onSettled: () => invalidateDots(qc),
    onError: (e) => {
      setRefused((n) => n + 1);
      notifyError(e);
    },
  });
  const value = save.isPending ? save.variables : dot.model;
  const id = `dot-model-switch-${dot.id}`;
  return (
    <div className="flex items-center gap-1.5" title={refusal ?? NEXT_REPLY}>
      <label htmlFor={id} className="text-xs text-muted-foreground">
        Next reply
      </label>
      <ModelPicker
        // A new picker when the dot's model changes, so a refused change goes back to it.
        key={`${dot.id}:${dot.model}:${refused}`}
        id={id}
        ariaLabel="Model for the next reply"
        compact
        commit
        // Off until it is known whether the person may edit the dot. If that cannot be read,
        // the core still refuses a change it does not allow.
        disabled={save.isPending || access.isPending || refusal !== undefined}
        value={value}
        options={models.options}
        onChange={(model) => save.mutate(model)}
      />
    </div>
  );
}
