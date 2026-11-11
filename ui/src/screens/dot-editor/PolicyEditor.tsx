import { Plus, Trash2 } from "lucide-react";
import { useEffect, useId, useState } from "react";

import { commands } from "@/api/commands";
import {
  Action,
  Preset,
  type EngineKind,
  type Guardrails,
  type Policy,
  type PolicyCeiling,
  type Rule,
} from "@/api/types";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { describeAllowRule, isDotsOwnRule } from "@/lib/allow-rules";
import { PRESET_SENTENCES } from "@/lib/dot-rules";
import { ceilingLine, GIT_LABELS, PRESET_NAMES, protectedCount } from "@/lib/guardrails";
import { notifyError } from "@/lib/notify";
import { ManagedPolicyNotice, useManagedCheck } from "@/screens/ManagedPolicyNotice";

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
  /// Only a dot run by the Claude CLI meets the company's Claude Code policy; default `claude`.
  engine?: EngineKind;
  /// The workspace's policy ceiling for this policy (guardrails spec section 4.3): presets past
  /// it are off with the core's reason, and a policy past it says why it can't be saved.
  ceiling?: PolicyCeiling;
  /// The stored dot's runs wait with this text: its policy is past the ceiling.
  held?: string;
}) {
  const { policy, onChange } = props;
  const managed = useManagedCheck(props.engine ?? "claude", policy);
  // Named presets keep their rules folded; a custom policy or a rule error shows them from the
  // start.
  const [open, setOpen] = useState(policy.preset === "custom" || Boolean(props.error));
  // The rules also open when they get an error, and when the policy turns custom from outside
  // (a template, a reset, the card), not only on mount.
  const [seen, setSeen] = useState({ preset: policy.preset, error: props.error });
  if (seen.preset !== policy.preset || seen.error !== props.error) {
    const turnedCustom = policy.preset === "custom" && seen.preset !== "custom";
    const newError = props.error !== undefined && props.error !== "" && props.error !== seen.error;
    if (turnedCustom || newError) setOpen(true);
    setSeen({ preset: policy.preset, error: props.error });
  }
  const custom = (change: Partial<Policy>) => onChange({ ...policy, ...change, preset: "custom" });
  const setRule = (index: number, change: Partial<Rule>) =>
    custom({ rules: policy.rules.map((r, i) => (i === index ? { ...r, ...change } : r)) });

  function choosePreset(preset: Preset) {
    if (preset === "custom") {
      setOpen(true);
      return custom({});
    }
    // A preset sets the rules; the person's own "Always ask before sending or deleting" stays.
    commands
      .policyPreset(preset)
      .then((p) => onChange({ ...p, ask_send_delete: policy.ask_send_delete }), notifyError);
  }

  const { ceiling } = props;
  const presetRefusal = (p: Preset) =>
    ceiling?.presets.find((entry) => entry.preset === p)?.refusal ?? undefined;
  // The presets this PC may not pick, each with the core's reason; the one in use is told by
  // `policy_refusal` instead.
  const refused = Preset.options.filter((p) => p !== policy.preset && presetRefusal(p));
  const refusedId = useId();
  const line = ceiling ? ceilingLine(ceiling) : null;
  return (
    <div className="flex flex-col gap-3">
      {props.held && (
        <p role="status" className="text-xs text-needs">
          {props.held}
        </p>
      )}
      {line && <p className="text-xs text-muted-foreground">{line}</p>}
      <Field label="Preset" htmlFor="policy-preset">
        <NativeSelect
          id="policy-preset"
          value={policy.preset}
          aria-describedby={refused.length > 0 ? refusedId : undefined}
          onChange={(e) => choosePreset(Preset.parse(e.target.value))}
        >
          {Preset.options.map((p) => (
            <option key={p} value={p} disabled={!!presetRefusal(p)} title={presetRefusal(p)}>
              {PRESETS[p]}
            </option>
          ))}
        </NativeSelect>
      </Field>
      {refused.length > 0 && (
        <ul id={refusedId} className="flex flex-col gap-0.5 text-xs text-muted-foreground">
          {refused.map((p) => (
            <li key={p}>
              {PRESET_NAMES[p]} is off: {presetRefusal(p)}
            </li>
          ))}
        </ul>
      )}
      {ceiling?.policy_refusal && (
        <p role="alert" className="text-xs text-destructive">
          {ceiling.policy_refusal}
        </p>
      )}
      {ceiling?.protected && (
        <ProtectedList
          guardrails={ceiling.protected}
          workspace={ceiling.workspace_name ?? "this workspace"}
        />
      )}
      <p className="text-xs text-muted-foreground">
        {policy.preset === "custom" ? "Your own rules." : PRESET_SENTENCES[policy.preset]}
      </p>
      <ManagedPolicyNotice check={managed} />
      <SendDeleteSwitch
        on={policy.ask_send_delete}
        onChange={(on) => onChange({ ...policy, ask_send_delete: on })}
      />
      <AlwaysAllowed
        rules={policy.rules}
        onRemove={(index) => custom({ rules: policy.rules.filter((_, j) => j !== index) })}
      />
      <details
        className="flex flex-col gap-3"
        open={open}
        onToggle={(e) => setOpen(e.currentTarget.open)}
      >
        <summary className="cursor-pointer text-sm font-medium">
          {open ? "Hide rules" : "Show rules"}
        </summary>
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
      </details>
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

/// What the workspace protects, folded under a count: brief, since the Guardrails tab of the
/// account has the whole of it.
function ProtectedList({ guardrails: g, workspace }: { guardrails: Guardrails; workspace: string }) {
  return (
    <details className="text-xs">
      <summary className="cursor-pointer text-muted-foreground">
        Protected in {workspace}: {protectedCount(g)}
      </summary>
      <dl className="mt-1 flex flex-col gap-1 pl-3">
        <dt className="font-medium">Paths</dt>
        <dd className="font-mono">{g.protected_paths.join(", ") || "None"}</dd>
        <dt className="font-medium">Git</dt>
        <dd>{g.protected_git.map((a) => GIT_LABELS[a]).join(", ") || "None"}</dd>
        <dt className="font-medium">Commands</dt>
        <dd className="font-mono">{g.protected_commands.join(", ") || "None"}</dd>
      </dl>
    </details>
  );
}

/// "Always ask before sending or deleting": on by default. It is not a rule, so it leaves the
/// preset as it is.
function SendDeleteSwitch({ on, onChange }: { on: boolean; onChange: (on: boolean) => void }) {
  const id = useId();
  return (
    <div className="flex flex-col gap-1 rounded-md border p-2">
      <label htmlFor={id} className="flex items-center gap-2 text-sm font-medium">
        <input
          id={id}
          type="checkbox"
          role="switch"
          aria-checked={on}
          checked={on}
          onChange={(e) => onChange(e.target.checked)}
        />
        Always ask before sending or deleting
      </label>
      <p className="text-xs text-muted-foreground">
        {on
          ? "Sending an email or message, posting, pushing, and deleting files or data always waits for your approval, even where a rule allows it. A deny rule still refuses."
          : "Off: sending and deleting follow the rules above like any other call."}
      </p>
    </div>
  );
}
