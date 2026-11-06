import type { Run } from "@/api/types";
import { autoLabel, modelLabel } from "@/lib/auto-model";

/// The model Auto picked for a run and why, under the dot's reply (auto model spec section 5):
/// "Auto · Sonnet — code work: refactor". Nothing for a run of a dot with its own model.
export function AutoLine({ run }: { run: Run }) {
  const label = autoLabel(run);
  if (label === null) return null;
  const tried =
    run.model_fallback_from === null
      ? undefined
      : `${modelLabel(run.model_fallback_from)} could not be used, so this reply used ${modelLabel(run.model ?? "")}.`;
  return (
    <p className="text-xs text-muted-foreground" title={tried}>
      {label}
      {tried && " · fallback"}
    </p>
  );
}
