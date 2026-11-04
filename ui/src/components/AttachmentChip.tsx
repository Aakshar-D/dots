import { cn } from "cn";
import { Paperclip, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { formatBytes } from "@/lib/format";

/// One attached file: its name and size (attachments spec section 6). `size` null: still being
/// added. With `onRemove`, a × removes it.
export function AttachmentChip(props: {
  name: string;
  size: number | null;
  onRemove?: () => void;
  className?: string;
}) {
  return (
    <span
      className={cn(
        "inline-flex max-w-full min-w-0 items-center gap-1.5 rounded-lg border bg-card px-2 py-1 text-xs text-card-foreground",
        props.className,
      )}
    >
      <Paperclip aria-hidden className="size-3 shrink-0 text-muted-foreground" />
      <span className="min-w-0 truncate" title={props.name}>
        {props.name}
      </span>
      <span className="shrink-0 text-muted-foreground">
        {props.size === null ? "Adding…" : formatBytes(props.size)}
      </span>
      {props.onRemove && (
        <Button
          size="icon-xs"
          variant="ghost"
          aria-label={`Remove ${props.name}`}
          title={`Remove ${props.name}`}
          onClick={props.onRemove}
        >
          <X aria-hidden />
        </Button>
      )}
    </span>
  );
}
