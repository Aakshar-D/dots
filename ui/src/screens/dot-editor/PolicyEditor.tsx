import { Plus, Trash2 } from "lucide-react";
import { useEffect, useId, useState } from "react";

import { commands } from "@/api/commands";
import { Action, Preset, type Policy, type Rule } from "@/api/types";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { describeAllowRule, isDotsOwnRule } from "@/lib/allow-rules";
import { notifyError } from "@/lib/notify";

const PRESETS: Record<Preset, string> = {
  "read-only": "Read-only: may read, never writes or runs commands",
  sandboxed: "Sandboxed: may edit inside its folder, asks for the rest",
  trusted: "Trusted: may do anything, asks before git push and MCP writes",
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
      <AlwaysAllowed
        rules={policy.rules}
        onRemove={(index) => custom({ rules: policy.rules.filter((_, j) => j !== index) })}
      />
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

/// The dot's allow rules in words, each with Remove: what it may do without asking. Dots' own
/// tools, which every preset allows, are left out. Removing a rule goes through the same change
/// as any rule edit, and is saved with the form.
function AlwaysAllowed(props: { rules: Rule[]; onRemove: (index: number) => void }) {
  const allowed = props.rules
    .map((rule, index) => ({ rule, index }))
    .filter(({ rule }) => rule.action === "allow" && rule.tool.trim() && !isDotsOwnRule(rule.tool));
  const labelId = useId();
  // Which of the rules cover a private folder, as the core reads them. A failed check leaves
  // every rule unmarked.
  const patterns = allowed.map(({ rule }) => rule.tool);
  const key = JSON.stringify(patterns);
  const [privateRules, setPrivateRules] = useState<Set<string>>(new Set());
  useEffect(() => {
    let current = true;
    const asked: string[] = JSON.parse(key);
    if (asked.length === 0) return;
    commands.rulesTouchPrivate(asked).then(
      (flags) => {
        if (current) setPrivateRules(new Set(asked.filter((_, i) => flags[i])));
      },
      () => {
        if (current) setPrivateRules(new Set());
      },
    );
    return () => {
      current = false;
    };
  }, [key]);
  return (
    <div className="flex flex-col gap-1.5">
      <span id={labelId} className="text-sm font-medium">
        Always allowed
      </span>
      {allowed.length === 0 ? (
        <p className="text-xs text-muted-foreground">Nothing is always allowed.</p>
      ) : (
        <ul aria-labelledby={labelId} className="flex flex-col gap-1">
          {allowed.map(({ rule, index }) => {
            const words = describeAllowRule(rule.tool);
            return (
              <li key={index} className="flex items-center gap-2 text-sm">
                <span className="min-w-0 flex-1">
                  {words} <code className="font-mono text-xs text-muted-foreground">{rule.tool}</code>
                  {privateRules.has(rule.tool) && (
                    <span className="ml-2 text-xs text-muted-foreground">
                      Always asks: private folder
                    </span>
                  )}
                </span>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  aria-label={`Remove ${words}`}
                  onClick={() => props.onRemove(index)}
                >
                  Remove
                </Button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
