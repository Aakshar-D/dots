import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { commands } from "@/api/commands";
import type { McpCatalog } from "@/api/types";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { NativeSelect } from "@/components/ui/native-select";
import { errorMessage } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

import { isDeclined, PERSONAL, useScopes, type ServerEntry } from "./parts";
import { variablesIn } from "./text";

/// What a Check found, in words.
function checked(name: string, catalog: McpCatalog): { text: string; failed: boolean } {
  switch (catalog.status) {
    case "accepted":
      return { text: `Checked as ${name}: the tools are the ones you accepted.`, failed: false };
    case "changed":
      return {
        text: `Checked as ${name}: the tools changed. Review them in the list.`,
        failed: false,
      };
    case "unreachable":
      return { text: `Checked as ${name}: ${catalog.error ?? "unreachable"}`, failed: true };
    case "unchecked":
      return { text: `Checked as ${name}.`, failed: false };
  }
}

/// A saved server's Check in a scope the person picks ("Check as": Personal or a joined org,
/// env vault spec §8), and, for a server whose URL reads a variable, the origins confirmed in
/// each scope, with "Confirm origin in <scope>" where there is none yet. Both ask in a native
/// dialog before a scope's values go to an origin not confirmed there; a no changes nothing. A
/// plugin's own server has these too, though its settings are the plugin's.
export function ServerScopes({ entry }: { entry: ServerEntry }) {
  const qc = useQueryClient();
  const { orgs, nameOf } = useScopes();
  // "" is Personal, which the commands take as a scope left out.
  const [picked, setPicked] = useState("");
  const scopeOf = (key: string) => (key === "" || key === PERSONAL ? undefined : key);
  const check = useMutation({
    mutationFn: async (scope: string | undefined) => ({
      scope,
      catalog: await commands.libraryCheck(entry.id, scope),
    }),
    onSuccess: () => invalidate(qc, keys.library),
  });
  const confirm = useMutation({
    mutationFn: (scope: string | undefined) => commands.confirmOrigin(entry.id, scope),
    onSuccess: () => invalidate(qc, keys.library),
  });

  const config = entry.config;
  const pinned = config.transport === "http" && variablesIn([config.url]).length > 0;
  const scopes = [PERSONAL, ...orgs.map((org) => org.id)];
  const result = check.data && checked(nameOf(check.data.scope), check.data.catalog);
  return (
    <div className="flex flex-col gap-2 rounded-md border p-3">
      <div className="flex items-end gap-3">
        <Field
          label="Check as"
          htmlFor="server-check-as"
          hint="Lists the tools with the values of this scope, as a run of a dot in it would."
        >
          <NativeSelect
            id="server-check-as"
            value={picked}
            onChange={(e) => {
              setPicked(e.target.value);
              check.reset();
            }}
          >
            <option value="">Personal</option>
            {orgs.map((org) => (
              <option key={org.id} value={org.id}>
                {org.name}
              </option>
            ))}
          </NativeSelect>
        </Field>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={check.isPending}
          onClick={() => check.mutate(scopeOf(picked))}
        >
          {check.isPending ? "Checking…" : "Check"}
        </Button>
      </div>
      {result && (
        <p role="status" className={`text-xs ${result.failed ? "text-destructive" : ""}`}>
          {result.text}
        </p>
      )}
      {check.isError &&
        (isDeclined(check.error) ? (
          <p role="status" className="text-xs text-muted-foreground">
            Not checked: you did not confirm the origin.
          </p>
        ) : (
          <p role="alert" className="text-xs text-destructive">
            {errorMessage(check.error)}
          </p>
        ))}

      {pinned && (
        <>
          <p className="text-xs text-muted-foreground">
            This server's URL reads a variable, so where it sends values can differ by scope.
            Dots sends a scope's values only to an origin confirmed in that scope.
          </p>
          <ul aria-label="Confirmed origins" className="flex flex-col gap-1 text-xs">
            {scopes.map((scope) => {
              const origins = entry.confirmed_origins[scope] ?? [];
              return (
                <li key={scope} aria-label={nameOf(scope)} className="flex flex-wrap gap-1">
                  <span className="font-medium">{`${nameOf(scope)}:`}</span>
                  {origins.length > 0 ? (
                    <span className="font-mono">{origins.join(", ")}</span>
                  ) : (
                    <span className="text-muted-foreground">None yet</span>
                  )}
                </li>
              );
            })}
          </ul>
          <div className="flex flex-wrap gap-2">
            {scopes
              .filter((scope) => (entry.confirmed_origins[scope] ?? []).length === 0)
              .map((scope) => (
                <Button
                  key={scope}
                  type="button"
                  variant="outline"
                  size="sm"
                  disabled={confirm.isPending}
                  onClick={() => confirm.mutate(scopeOf(scope))}
                >
                  {`Confirm origin in ${nameOf(scope)}`}
                </Button>
              ))}
          </div>
          {confirm.isError &&
            (isDeclined(confirm.error) ? (
              <p role="status" className="text-xs text-muted-foreground">
                You declined. Nothing was confirmed.
              </p>
            ) : (
              <p role="alert" className="text-xs text-destructive">
                {errorMessage(confirm.error)}
              </p>
            ))}
        </>
      )}
    </div>
  );
}
