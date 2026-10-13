import { Plus, Trash2 } from "lucide-react";

import { commands } from "@/api/commands";
import { Action, Preset, type Policy, type Rule } from "@/api/types";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { notifyError } from "@/lib/notify";

const PRESETS: Record<Preset, string> = {
  "read-only": "Read-only: may read, never writes or runs commands",
  sandboxed: "Sandboxed: may edit inside its folder, asks for the rest",
  trusted: "Trusted: may do anything, asks before git push and MCP tools",
  custom: "Custom: the rules below",
};

/// The dot's permission policy: a preset, or its own rules. Editing a rule or the default
/// makes the policy `custom`; choosing a preset replaces the rules with that preset's.
export function PolicyEditor(props: {
  policy: Policy;
  onChange: (policy: Policy) => void;
  error?: string;
}) {
  const { policy, onChange } = props;
  const custom = (change: Partial<Policy>) => onChange({ ...policy, ...change, preset: "custom" });
  const setRule = (index: number, change: Partial<Rule>) =>
    custom({ rules: policy.rules.map((r, i) => (i === index ? { ...r, ...change } : r)) });

  function choosePreset(preset: Preset) {
    if (preset === "custom") return custom({});
    commands.policyPreset(preset).then(onChange, notifyError);
  }

  return (
    <div className="flex flex-col gap-3">
      <Field label="Preset" htmlFor="policy-preset">
        <NativeSelect
          id="policy-preset"
          value={policy.preset}
          onChange={(e) => choosePreset(Preset.parse(e.target.value))}
        >
          {Preset.options.map((p) => (
            <option key={p} value={p}>
              {PRESETS[p]}
            </option>
          ))}
        </NativeSelect>
      </Field>
      <div className="flex flex-col gap-1.5">
        <span className="text-sm font-medium">Rules</span>
        <p className="text-xs text-muted-foreground">
          A rule names a tool, optionally with a pattern: <code>Bash(git status:*)</code>,{" "}
          <code>Read(src/**)</code>, <code>mcp__github__*</code>. Deny wins over ask, and ask over
          allow.
        </p>
        {policy.rules.map((rule, i) => (
          <div key={i} className="flex items-center gap-2">
            <Input
              aria-label={`Rule ${i + 1} tool`}
              className="font-mono"
              value={rule.tool}
              onChange={(e) => setRule(i, { tool: e.target.value })}
            />
            <NativeSelect
              aria-label={`Rule ${i + 1} action`}
              value={rule.action}
              onChange={(e) => setRule(i, { action: Action.parse(e.target.value) })}
            >
              {Action.options.map((a) => (
                <option key={a}>{a}</option>
              ))}
            </NativeSelect>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              aria-label={`Remove rule ${i + 1}`}
              onClick={() => custom({ rules: policy.rules.filter((_, j) => j !== i) })}
            >
              <Trash2 aria-hidden />
            </Button>
          </div>
        ))}
        {props.error && <p className="text-xs text-destructive">{props.error}</p>}
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="self-start"
          onClick={() => custom({ rules: [...policy.rules, { tool: "", action: "ask" }] })}
        >
          <Plus aria-hidden /> Add rule
        </Button>
      </div>
      <Field
        label="Anything else"
        htmlFor="policy-default"
        hint="What happens to a tool call no rule matches."
      >
        <NativeSelect
          id="policy-default"
          value={policy.default}
          onChange={(e) => custom({ default: Action.parse(e.target.value) })}
        >
          {Action.options.map((a) => (
            <option key={a}>{a}</option>
          ))}
        </NativeSelect>
      </Field>
    </div>
  );
}
