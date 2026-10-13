// The dot editor's checks. They mirror `DotSpec::validate` in the core (model.rs) so most
// mistakes show next to their field before a save; the core still has the last word.
import { z } from "zod";

import { LIMITS, type DotSpec } from "@/api/types";

export type SpecErrors = Partial<Record<keyof DotSpec, string>>;

const filled = (message: string) => z.string().refine((s) => s.trim() !== "", message);
const whole = (min: number, max: number, message: string) =>
  z.number({ error: message }).int(message).min(min, message).max(max, message);

const Fields = z.object({
  name: z.string().regex(/^[A-Za-z0-9_-]{1,64}$/, "Use 1 to 64 letters, digits, '-' or '_'."),
  instructions: filled("Say what the dot should do."),
  model: filled("Name the model to use."),
  workdir: filled("Choose the folder the dot works in."),
  max_turns: whole(1, Number.MAX_SAFE_INTEGER, "Max turns must be a whole number, 1 or more."),
  timeout_secs: whole(
    1,
    LIMITS.max_timeout_secs,
    `Timeout must be a whole number of seconds, 1 to ${LIMITS.max_timeout_secs} (7 days).`,
  ),
  approval_wait_secs: whole(
    0,
    LIMITS.max_approval_wait_secs,
    `Approval wait must be a whole number of seconds, 0 to ${LIMITS.max_approval_wait_secs}.`,
  ),
});

/// The MCP servers a text field holds. An empty field is "none".
export function parseMcp(
  text: string,
): { value: Record<string, unknown> | null } | { error: string } {
  if (text.trim() === "") return { value: null };
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return { error: "This is not valid JSON." };
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    return { error: "Write a JSON object: server name, then its configuration." };
  }
  if ("dots" in parsed) return { error: "The server name 'dots' is reserved." };
  return { value: parsed as Record<string, unknown> };
}

/// The MCP servers a draft saves. Only a Claude dot has them: a local dot's form hides the
/// field, so what it still holds is neither checked nor saved.
export function mcpOf(spec: DotSpec, mcpText: string): ReturnType<typeof parseMcp> {
  return spec.engine === "claude" ? parseMcp(mcpText) : { value: null };
}

/// What is wrong with a draft, by field. Empty when it can be saved.
export function specErrors(spec: DotSpec, mcpText: string): SpecErrors {
  const errors: SpecErrors = {};
  const fields = Fields.safeParse(spec);
  if (!fields.success) {
    for (const issue of fields.error.issues) {
      errors[issue.path[0] as keyof DotSpec] ??= issue.message;
    }
  }
  if (spec.engine === "local") {
    const url = (spec.endpoint_url ?? "").trim();
    if (url.startsWith("https://")) {
      errors.endpoint_url = "Use http://. https is not supported for local endpoints.";
    } else if (!/^http:\/\/[^\s/]+/.test(url)) {
      errors.endpoint_url = "A local dot needs its endpoint, starting with http://.";
    }
  }
  if (spec.policy.rules.some((r) => r.tool.trim() === "")) {
    errors.policy = "Every rule needs a tool pattern.";
  }
  const mcp = mcpOf(spec, mcpText);
  if ("error" in mcp) errors.mcp_servers = mcp.error;
  return errors;
}

/// The editable part of a stored dot.
export function specOf(dot: DotSpec): DotSpec {
  return {
    name: dot.name,
    instructions: dot.instructions,
    engine: dot.engine,
    model: dot.model,
    endpoint_url: dot.endpoint_url,
    workdir: dot.workdir,
    workspace_mode: dot.workspace_mode,
    schedule: dot.schedule,
    policy: dot.policy,
    mcp_servers: dot.mcp_servers,
    use_user_settings: dot.use_user_settings,
    max_turns: dot.max_turns,
    timeout_secs: dot.timeout_secs,
    approval_wait_secs: dot.approval_wait_secs,
    enabled: dot.enabled,
  };
}
