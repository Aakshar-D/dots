import { keepPreviousData, useMutation, useQuery } from "@tanstack/react-query";
import { Copy, Save } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";

import { commands } from "@/api/commands";
import { ErrorNote } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { notifyError } from "@/lib/notify";
import { useDebounced } from "@/lib/use-debounced";

/// Report a problem (report a problem spec §2): the person's words, whether to include recent
/// log lines, and the exact text that is copied or saved. Nothing is sent anywhere.
export function ReportProblemDialog(props: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [description, setDescription] = useState("");
  const [includeLogs, setIncludeLogs] = useState(true);
  const settled = useDebounced(description, 300);
  const report = useQuery({
    queryKey: ["problem-report", settled, includeLogs] as const,
    queryFn: () => commands.problemReport(settled, includeLogs),
    enabled: props.open,
    placeholderData: keepPreviousData,
    // The log grows while the dialog is open; the preview changes only when asked to.
    staleTime: Infinity,
    gcTime: 0,
  });
  const text = report.data;
  const save = useMutation({
    mutationFn: (t: string) => commands.saveProblemReport(t),
    onSuccess: (path) => {
      if (path) toast.success(`Saved to ${path}`);
    },
    onError: notifyError,
  });
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Report a problem</DialogTitle>
          <DialogDescription>
            Dots does not send this anywhere. Copy it or save it, then send it yourself. Secrets,
            email addresses and your home folder are taken out of the log lines.
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-2">
          <Label htmlFor="report-description">What went wrong?</Label>
          <Textarea
            id="report-description"
            rows={3}
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
        </div>
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={includeLogs}
            onChange={(e) => setIncludeLogs(e.target.checked)}
          />
          Include recent log lines
        </label>
        {report.isError && <ErrorNote error={report.error} />}
        <div className="flex flex-col gap-2">
          <Label htmlFor="report-preview">What will be shared</Label>
          <Textarea
            id="report-preview"
            readOnly
            rows={12}
            className="max-h-80 font-mono text-xs [field-sizing:fixed]"
            value={text ?? ""}
          />
        </div>
        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            disabled={text === undefined}
            onClick={() =>
              text !== undefined &&
              navigator.clipboard
                .writeText(text)
                .then(() => toast.success("Copied the report"), notifyError)
            }
          >
            <Copy aria-hidden /> Copy
          </Button>
          <Button
            type="button"
            disabled={text === undefined || save.isPending}
            onClick={() => text !== undefined && save.mutate(text)}
          >
            <Save aria-hidden /> Save to file…
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
