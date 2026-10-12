import type { FileDiff, WorktreeDiff } from "@/api/types";
import { Badge } from "@/components/ui/badge";

function lineClass(line: string): string {
  if (line.startsWith("@@")) return "text-sky-300";
  if (line.startsWith("+")) return "bg-emerald-500/15 text-emerald-200";
  if (line.startsWith("-")) return "bg-red-500/15 text-red-200";
  if (line.startsWith("\\")) return "text-muted-foreground italic";
  return "";
}

/// The hunks of a patch: everything from the first `@@` line on. The header lines above it
/// repeat what the file's row already says.
export function hunkLines(patch: string): string[] {
  const lines = patch.split("\n");
  if (lines.at(-1) === "") lines.pop();
  const first = lines.findIndex((l) => l.startsWith("@@"));
  return first === -1 ? [] : lines.slice(first);
}

function File({ file }: { file: FileDiff }) {
  const lines = hunkLines(file.patch);
  return (
    <details open className="rounded-md border">
      <summary className="flex cursor-pointer items-center gap-2 px-2 py-1">
        <Badge variant="secondary">{file.status}</Badge>
        <span className="font-mono text-xs">{file.path}</span>
      </summary>
      {file.binary ? (
        <p className="border-t px-2 py-1 text-xs text-muted-foreground">Binary file.</p>
      ) : lines.length === 0 ? (
        <p className="border-t px-2 py-1 text-xs text-muted-foreground">
          No text changes to show.
        </p>
      ) : (
        <pre className="overflow-x-auto border-t text-xs">
          {lines.map((line, i) => (
            <div key={i} className={`px-2 ${lineClass(line)}`}>
              {line === "" ? " " : line}
            </div>
          ))}
        </pre>
      )}
    </details>
  );
}

/// What a thread changed in its worktree, one collapsible block per file.
export function DiffView({ diff }: { diff: WorktreeDiff }) {
  if (diff.files.length === 0) {
    return <p className="text-muted-foreground">No changes.</p>;
  }
  return (
    <div className="flex flex-col gap-2">
      {diff.truncated && (
        <p role="status" className="rounded-md border border-amber-500/40 bg-amber-500/15 p-2">
          This diff is larger than 1 MiB. Every changed file is listed, but some are shown
          without their changes. Open the folder to see them.
        </p>
      )}
      {diff.files.map((file) => (
        <File key={file.path} file={file} />
      ))}
    </div>
  );
}
