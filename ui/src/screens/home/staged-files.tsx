import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";

import { commands, toCommandError } from "@/api/commands";
import type { StagedAttachment } from "@/api/types";
import { MAX_FILE_BYTES, pastedName, room, TOO_MANY } from "@/lib/attachments";
import type { ComposerFile } from "@/screens/home/Composer";

/// A file in the chat box: its chip, and its staged id once the core has it.
type Staged = ComposerFile & { id: string | null };

/// What the chat box offers to add, or why it refuses it before asking the core.
type Adding = { name: string; stage: () => Promise<StagedAttachment> } | { refused: string };

/// The files in a dot's chat box (attachments spec sections 3 and 6): each is staged by the
/// core the moment it is added. Refusals say why a file was not added; they clear on the next
/// add or a send. The files belong to the dot pane, not to the topic it shows, so moving
/// between the conversation and a History topic keeps them. Files the box still holds when the
/// dot pane closes are removed from the core, except those a send in flight carries.
function useStagedFiles(dotId: string) {
  const [files, setFiles] = useState<Staged[]>([]);
  const [refusals, setRefusals] = useState<string[]>([]);
  const current = useRef<Staged[]>([]);
  // Ids that a send or a schedule on its way carries: never removed under it.
  const sending = useRef(new Set<string>());
  const next = useRef(0);
  const mounted = useRef(true);
  const update = (change: (files: Staged[]) => Staged[]) => {
    current.current = change(current.current);
    setFiles(current.current);
  };
  useEffect(() => {
    mounted.current = true;
    const carried = sending.current;
    return () => {
      mounted.current = false;
      for (const f of current.current) {
        if (f.id !== null && !carried.has(f.id)) {
          void commands.attachmentRemove(f.id).catch(() => {});
        }
      }
      current.current = [];
    };
  }, []);
  function add(items: Adding[]) {
    const refused = items.flatMap((item) => ("refused" in item ? [item.refused] : []));
    const stageable = items.flatMap((item) => ("refused" in item ? [] : [item]));
    const fit = room(current.current.length, stageable.length);
    setRefusals(fit.refused ? [TOO_MANY, ...refused] : refused);
    for (const item of stageable.slice(0, fit.take)) {
      const key = `f${next.current++}`;
      update((fs) => [...fs, { key, name: item.name, size: null, id: null }]);
      item.stage().then(
        (staged) => {
          if (!mounted.current) {
            void commands.attachmentRemove(staged.id).catch(() => {});
            return;
          }
          update((fs) =>
            fs.map((f) =>
              f.key === key ? { key, name: staged.name, size: staged.size, id: staged.id } : f,
            ),
          );
        },
        (error: unknown) => {
          if (!mounted.current) return;
          update((fs) => fs.filter((f) => f.key !== key));
          setRefusals((rs) => [...rs, toCommandError(error).message]);
        },
      );
    }
  }
  return {
    files,
    refusals,
    ids: () => current.current.flatMap((f) => (f.id === null ? [] : [f.id])),
    addPaths: (paths: string[]) =>
      add(
        paths.map((path) => ({
          name: path.split(/[\\/]/).pop() ?? path,
          stage: () => commands.attachmentAddPath(dotId, path),
        })),
      ),
    // A pasted file over the limit is refused by its size, before its bytes are read: a
    // pasted video of gigabytes is never read into memory or sent to the core.
    addFiles: (pasted: File[]) =>
      add(
        pasted.map((file): Adding => {
          const name = pastedName(file);
          if (file.size > MAX_FILE_BYTES) return { refused: `${name} is larger than 25 MB.` };
          return {
            name,
            stage: async () =>
              commands.attachmentAddBytes(dotId, name, new Uint8Array(await file.arrayBuffer())),
          };
        }),
      ),
    remove: (key: string) => {
      const f = current.current.find((x) => x.key === key);
      if (f === undefined || f.id === null) return;
      update((fs) => fs.filter((x) => x.key !== key));
      void commands.attachmentRemove(f.id).catch(() => {});
    },
    /// A send or schedule takes `ids`: they are not removed while it is on its way.
    sending: (ids: string[]) => {
      for (const id of ids) sending.current.add(id);
    },
    /// The send of `ids` is over. When it failed and the box has gone, they are removed.
    settled: (ids: string[], ok: boolean) => {
      for (const id of ids) sending.current.delete(id);
      if (!ok && !mounted.current) {
        for (const id of ids) void commands.attachmentRemove(id).catch(() => {});
      }
    },
    /// The files went with a message: the box is empty again.
    sent: () => {
      update(() => []);
      setRefusals([]);
    },
    /// Says `text` under the chips; false when the box has gone, so the caller says it.
    refuse: (text: string) => {
      if (!mounted.current) return false;
      setRefusals([text]);
      return true;
    },
    clearRefusals: () => setRefusals([]),
  };
}

export type StagedFiles = ReturnType<typeof useStagedFiles>;

const Context = createContext<StagedFiles | null>(null);

/// Holds a dot's staged files for as long as its pane is open. Keyed by the dot by its parent.
export function StagedFilesProvider(props: { dotId: string; children: ReactNode }) {
  const staged = useStagedFiles(props.dotId);
  return <Context.Provider value={staged}>{props.children}</Context.Provider>;
}

/// The staged files of the dot pane this is in.
export function useStaged(): StagedFiles {
  const staged = useContext(Context);
  if (staged === null) throw new Error("useStaged needs a StagedFilesProvider");
  return staged;
}
