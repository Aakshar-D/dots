import { History } from "lucide-react";
import { DropdownMenu } from "radix-ui";
import { useNavigate } from "react-router";

import { Button } from "@/components/ui/button";
import { localDateTime } from "@/lib/format";
import { topicTitle, type Topic } from "@/lib/home-topics";

const ITEM =
  "flex cursor-default flex-col rounded-sm px-2 py-1.5 outline-none select-none data-[disabled]:text-muted-foreground data-[highlighted]:bg-accent";

/// The dot's topics older than the conversation, newest first, each opening alone.
export function HistoryMenu({ dotId, topics }: { dotId: string; topics: Topic[] }) {
  const navigate = useNavigate();
  return (
    <DropdownMenu.Root modal={false}>
      <DropdownMenu.Trigger asChild>
        <Button size="sm" variant="ghost">
          <History aria-hidden /> History
        </Button>
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content
          align="end"
          className="z-50 max-h-96 w-72 overflow-auto rounded-md border bg-popover p-1 text-sm text-popover-foreground shadow-md"
        >
          {topics.length === 0 && (
            <DropdownMenu.Item disabled className={ITEM}>
              No older topics
            </DropdownMenu.Item>
          )}
          {topics.map((topic) => (
            <DropdownMenu.Item
              key={topic.rootId}
              className={ITEM}
              onSelect={() => void navigate(`/home/${dotId}/${topic.rootId}`)}
            >
              <span className="truncate">{topicTitle(topic)}</span>
              <span className="text-xs text-muted-foreground">
                {localDateTime(topic.root.queued_at)}
              </span>
            </DropdownMenu.Item>
          ))}
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}
