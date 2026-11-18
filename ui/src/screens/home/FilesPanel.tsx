import { cn } from "cn";
import { FileText, FolderOpen } from "lucide-react";
import { useId } from "react";

import { commands } from "@/api/commands";
import type { ConversationFile } from "@/api/types";
import { Button } from "@/components/ui/button";
import { localDayTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";

/// Why a listed file has no Open (attachments spec section 4.5.1).
const NOT_OPENED = "Sage only opens documents, images and media. Use Show in folder.";

/// The Files panel beside the conversation (attachments spec section 6): the files the person
/// attached and the files the dot wrote or edited, newest first. A name opens its file, the
/// folder button shows it in Explorer. The core resolves both from the list it made, so a path
/// is never opened on the UI's word. Below the `md` breakpoint it is not drawn.
export function FilesPanel(props: {
  dotId: string;
  dotName: string;
  roots: string[];
  files: ConversationFile[];
}) {
  const { dotId, roots } = props;
  const ids = useId();
  const openFile = (path: string) =>
    commands.conversationFileOpen(dotId, roots, path).catch(notifyError);
  const reveal = (path: string) =>
    commands.conversationFileReveal(dotId, roots, path).catch(notifyError);
  return (
    <aside
      aria-label="Files"
      className="hidden w-72 shrink-0 flex-col gap-2 overflow-y-auto border-l px-3 py-3 md:flex"
    >
      <h2 className="px-1 text-sm font-semibold">Files</h2>
      <ul className="flex flex-col gap-1">
        {props.files.map((f, i) => {
          const what = f.source === "attached" ? "Attached" : `Changed by ${props.dotName}`;
          const notOpened = f.exists && !f.can_open;
          const why = `${ids}-why-${i}`;
          return (
            <li
              key={f.path}
              className="flex items-start gap-1.5 rounded-lg px-1 py-1 hover:bg-muted"
            >
              <FileText aria-hidden className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
              <div className="flex min-w-0 flex-1 flex-col">
                {f.exists && f.can_open ? (
                  <button
                    type="button"
                    title={f.path}
                    className="truncate text-left text-sm hover:underline"
                    onClick={() => void openFile(f.path)}
                  >
                    {f.name}
                  </button>
                ) : (
                  <span
                    title={f.exists ? NOT_OPENED : f.path}
                    className={cn("truncate text-sm", !f.exists && "text-muted-foreground")}
                  >
                    {f.name}
                  </span>
                )}
                <span className="text-xs text-muted-foreground">
                  {f.exists ? `${what} · ${localDayTime(f.at)}` : "Gone"}
                </span>
                {notOpened && (
                  <span id={why} className="text-xs text-muted-foreground">
                    {NOT_OPENED}
                  </span>
                )}
              </div>
              {f.exists && (
                <Button
                  size="icon-xs"
                  variant="ghost"
                  aria-label={`Show ${f.name} in folder`}
                  aria-describedby={notOpened ? why : undefined}
                  title="Show in folder"
                  onClick={() => void reveal(f.path)}
                >
                  <FolderOpen aria-hidden />
                </Button>
              )}
            </li>
          );
        })}
      </ul>
    </aside>
  );
}

/// The header's toggle of the Files panel: pressed while the panel shows. Like the panel, it is
/// not drawn below the `md` breakpoint.
export function FilesToggle(props: { hidden: boolean; onToggle: () => void }) {
  return (
    <Button
      size="sm"
      variant="ghost"
      aria-pressed={!props.hidden}
      className="hidden md:inline-flex"
      title={props.hidden ? "Show files" : "Hide files"}
      onClick={props.onToggle}
    >
      Files
    </Button>
  );
}
