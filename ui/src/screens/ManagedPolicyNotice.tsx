import { useEffect, useState } from "react";

import { commands } from "@/api/commands";
import type { EngineKind, ManagedCheck, Policy, Preset } from "@/api/types";
import { describeAllowRule } from "@/lib/allow-rules";

/// Where `policy` of a dot run by `engine` runs into the company's Claude Code policy. `null`
/// until known, and when it cannot be read (the notice is then simply not shown). Only a dot
/// run by the Claude CLI is checked.
export function useManagedCheck(engine: EngineKind, policy: Policy | null): ManagedCheck | null {
  const [check, setCheck] = useState<ManagedCheck | null>(null);
  const key = engine === "claude" && policy ? JSON.stringify(policy) : null;
  useEffect(() => {
    if (key === null) {
      setCheck(null);
      return;
    }
    let live = true;
    commands.managedPolicyCheck("claude", JSON.parse(key) as Policy).then(
      (c) => live && setCheck(c),
      () => live && setCheck(null),
    );
    return () => {
      live = false;
    };
  }, [key]);
  return check;
}

/// The same for a preset, as the setup card proposes one.
export function useManagedCheckForPreset(preset: Preset): ManagedCheck | null {
  const [policy, setPolicy] = useState<Policy | null>(null);
  useEffect(() => {
    let live = true;
    commands.policyPreset(preset).then(
      (p) => live && setPolicy(p),
      () => live && setPolicy(null),
    );
    return () => {
      live = false;
    };
  }, [preset]);
  return useManagedCheck("claude", policy);
}

/// Says, in plain words, what the company's policy refuses whatever this dot's rules say.
/// Nothing when no deny rule conflicts. A managed ask rule, or managed-rules-only, gets one calm
/// line: Dots answers the CLI's permission prompts with the dot's own rules.
export function ManagedPolicyNotice(props: { check: ManagedCheck | null }) {
  const { check } = props;
  if (!check || check.conflicts.length === 0) return null;
  const { managed } = check;
  const asks = managed.forced_ask.length > 0 || managed.managed_rules_only;
  return (
    <div role="note" className="flex flex-col gap-1 rounded-md border p-3 text-xs text-needs">
      <p>
        Your company&apos;s Claude Code policy refuses these for every sol, whatever this
        sol&apos;s rules say:
      </p>
      <ul className="list-disc pl-5">
        {check.conflicts.map((c) => (
          <li key={c.rule}>{describeAllowRule(c.rule)}</li>
        ))}
      </ul>
      {asks && (
        <p className="text-muted-foreground">
          Your company&apos;s policy also asks before some actions; in Sage, those requests go to
          Sage&apos;s own approval check, which follows this sol&apos;s rules.
        </p>
      )}
      {managed.path && (
        <p>
          Policy file: <span className="font-mono break-all">{managed.path}</span>
        </p>
      )}
    </div>
  );
}
