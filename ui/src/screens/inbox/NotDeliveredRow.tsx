import type { ReactNode } from "react";
import { Link } from "react-router";

import type { UndeliveredMessage } from "@/api/types";
import { DeliveryLine } from "@/components/DeliveryLine";
import { Button } from "@/components/ui/button";
import { chatText, oneLine } from "@/lib/home-topics";

/// A chat message that did not reach its dot (snooze and done spec section 3.9.7): the dot, the
/// message's first line, the delivery line with Retry and Discard, and Open.
export function NotDeliveredRow({ item, actions }: { item: UndeliveredMessage; actions?: ReactNode }) {
  return (
    <li
      aria-label={`${item.dot_name} didn't get your message`}
      className="flex items-start gap-3 rounded-lg border p-3"
    >
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <span className="font-medium">{item.dot_name}</span>
        <span className="truncate text-xs text-muted-foreground">{oneLine(chatText(item))}</span>
        <DeliveryLine run={item} />
      </div>
      <Button asChild size="sm" variant="outline">
        <Link to={`/chat/${item.dot_id}/${item.id}`}>Open</Link>
      </Button>
      {actions}
    </li>
  );
}
