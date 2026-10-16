import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { ReactNode } from "react";

import { commands } from "@/api/commands";
import type { LibraryEntry } from "@/api/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { errorMessage } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

import { ToolList } from "./parts";
import { diffTools, parseTools } from "./tools";

function Group(props: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-1.5">
      <h3 className="text-xs font-semibold uppercase text-muted-foreground">{props.title}</h3>
      {props.children}
    </section>
  );
}

/// The review of a server whose tool list differs from the accepted one: the tools added,
/// removed and changed (and which part of a changed tool), and "Accept". Accepting names the
/// list that is shown; the core refuses it if the list moved meanwhile.
export function ToolsReview(props: { entry: LibraryEntry; onClose: () => void }) {
  const { entry } = props;
  const qc = useQueryClient();
  const catalog = entry.catalog;
  const diff = diffTools(parseTools(catalog?.accepted_tools), parseTools(catalog?.seen_tools));
  const accept = useMutation({
    mutationFn: () => commands.libraryAccept(entry.id, catalog?.seen_fingerprint ?? ""),
    onSuccess: () => {
      invalidate(qc, keys.library);
      props.onClose();
    },
    // The list moved since it was loaded: load the newer one.
    onError: () => invalidate(qc, keys.library),
  });
  const never = catalog?.accepted_fingerprint == null;

  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{`Tools changed: ${entry.name}`}</DialogTitle>
          <DialogDescription>
            {never
              ? "This server's tools were never accepted."
              : "The server lists different tools than the ones you accepted."}{" "}
            Until you accept the new list, runs that use the server ask before every call to it.
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-4">
          {diff.added.length > 0 && (
            <Group title="Added">
              <ToolList tools={diff.added} />
            </Group>
          )}
          {diff.removed.length > 0 && (
            <Group title="Removed">
              <ToolList tools={diff.removed} />
            </Group>
          )}
          {diff.changed.length > 0 && (
            <Group title="Changed">
              <ul className="flex flex-col gap-1.5">
                {diff.changed.map(({ tool, parts }) => (
                  <li key={tool.name} className="flex flex-col gap-1 rounded-md border px-2 py-1.5">
                    <span className="font-mono text-xs font-medium">{tool.name}</span>
                    <span className="flex flex-wrap gap-1">
                      {parts.map((part) => (
                        <Badge key={part} variant="secondary">{`${part} changed`}</Badge>
                      ))}
                    </span>
                  </li>
                ))}
              </ul>
            </Group>
          )}
          {accept.isError && (
            <p role="alert" className="text-xs text-destructive">
              {errorMessage(accept.error)}
            </p>
          )}
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={props.onClose}>
            Close
          </Button>
          <Button type="button" disabled={accept.isPending} onClick={() => accept.mutate()}>
            Accept
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
