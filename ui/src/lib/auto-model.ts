import type { Run } from "@/api/types";

/// The model id a dot stores for Auto (`dots_core::auto_model::AUTO_MODEL`).
export const AUTO_MODEL = "auto";

/// Whether `model` names Auto, as the core reads it: any case, spaces around it ignored.
export const isAuto = (model: string) => model.trim().toLowerCase() === AUTO_MODEL;

/// The curated Claude names (`dots_core::models::CLAUDE_MODELS`).
const LABELS: Record<string, string> = { opus: "Opus", sonnet: "Sonnet", haiku: "Haiku" };

/// A model's name as the person knows it: "Sonnet" for `sonnet`, else the id itself.
export const modelLabel = (model: string) => LABELS[model.trim().toLowerCase()] ?? model.trim();

/// The words beside a reply Auto picked the model of (auto model spec section 5):
/// "Auto · Sonnet — code work: refactor". Null for a run Auto did not pick.
export function autoLabel(run: Pick<Run, "model" | "model_reason">): string | null {
  if (run.model === null || run.model_reason === null) return null;
  return `Auto · ${modelLabel(run.model)} — ${run.model_reason}`;
}

/// What a run's details say about its model: Auto's label and, after a fallback, the model it
/// tried first; or the model alone for a dot with its own. Null before the run starts.
export function modelLine(
  run: Pick<Run, "model" | "model_reason" | "model_fallback_from">,
): string | null {
  if (run.model === null) return null;
  const auto = autoLabel(run);
  if (auto === null) return modelLabel(run.model);
  return run.model_fallback_from === null
    ? auto
    : `${auto} (${modelLabel(run.model_fallback_from)} could not be used)`;
}
