import { Copy } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { notifyError } from "@/lib/notify";

/// The request code a waiting PC sends to an Admin of `account` (accounts spec §6.4): shown to
/// read and to copy, with where it goes and when Dots looks again.
export function RequestCodeBox({ account, code }: { account: string; code: string }) {
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center gap-2">
        <Input readOnly aria-label="Request code" className="font-mono text-xs" value={code} />
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() =>
            navigator.clipboard
              .writeText(code)
              .then(() => toast.success("Copied the request code"), notifyError)
          }
        >
          <Copy aria-hidden /> Copy request code
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        Send this to an Admin of {account}. Dots checks again on its normal refresh.
      </p>
    </div>
  );
}
