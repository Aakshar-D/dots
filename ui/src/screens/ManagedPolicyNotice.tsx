import { useEffect, useState } from "react";

import { commands } from "@/api/commands";
import type { EngineKind, ManagedCheck, ManagedConflict, Policy, Preset } from "@/api/types";
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

function what(c: ManagedConflict): string {
  return c.rule === "" ? "everything this dot is allowed" : describeAllowRule(c.rule);
}

/// Says, in plain words, what the company's policy keeps asking about or refuses. Nothing when
/// there is no conflict.
export function ManagedPolicyNotice(props: { check: ManagedCheck | null }) {
  const { check } = props;
  if (!check || check.conflicts.length === 0) return null;
  const path = check.managed.path;
  const only = check.conflicts.some((c) => c.kind === "managed_rules_only");
  const rules = check.conflicts.filter((c) => c.kind !== "managed_rules_only");
  return (
    <div role="note" className="flex flex-col gap-1 rounded-md border p-3 text-xs text-needs">
      <p>
        Your company&apos;s Claude Code policy keeps asking turned on, so this dot will still ask
        (or be refused)
        {rules.length > 0 ? " for:" : "."}
      </p>
      {rules.length > 0 && (
        <ul className="list-disc pl-5">
          {rules.map((c) => (
            <li key={`${c.kind}/${c.rule}`}>
              {what(c)} {c.kind === "deny" ? "(refused)" : "(asks)"}
            </li>
          ))}
        </ul>
      )}
      {only && (
        <p>The policy also ignores the allow rules set here: only the company&apos;s rules count.</p>
      )}
      {path && (
        <p>
          Policy file: <span className="font-mono break-all">{path}</span>
        </p>
      )}
    </div>
  );
}
