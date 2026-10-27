import { useQuery } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import { ErrorNote } from "@/components/ErrorNote";
import { Markdown } from "@/components/Markdown";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { keys } from "@/lib/query";

/// A version's notes from its GitHub release (updates UI spec section 4.1).
export function WhatsNewDialog(props: {
  version: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const notes = useQuery({
    queryKey: keys.releaseNotes(props.version),
    queryFn: () => commands.updateReleaseNotes(props.version),
    enabled: props.open,
  });
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange}>
      <DialogContent aria-describedby={undefined} className="max-h-[80vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>What's new in {props.version}</DialogTitle>
        </DialogHeader>
        {notes.isError && <ErrorNote error={notes.error} />}
        {notes.data === null && (
          <p className="text-sm text-muted-foreground">No notes for {props.version}.</p>
        )}
        {notes.data && <Markdown text={notes.data.body} />}
      </DialogContent>
    </Dialog>
  );
}
