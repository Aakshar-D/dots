import { useQuery } from "@tanstack/react-query";
import { Copy } from "lucide-react";
import { toast } from "sonner";

import { commands } from "@/api/commands";
import { Button } from "@/components/ui/button";
import { notifyError } from "@/lib/notify";
import { keys } from "@/lib/query";

/// Copies this install's Dots ID (its public key) to the clipboard, to send to an account's
/// admin. Shown in Settings → Profile and on an account that waits for the member list.
export function CopyDotsIdButton() {
  const identity = useQuery({ queryKey: keys.identity, queryFn: commands.identityGet }).data;
  return (
    <Button
      type="button"
      variant="outline"
      size="sm"
      disabled={!identity}
      onClick={() =>
        identity &&
        navigator.clipboard
          .writeText(identity.public_key)
          .then(() => toast.success("Copied your Dots ID"), notifyError)
      }
    >
      <Copy aria-hidden /> Copy my Dots ID
    </Button>
  );
}
