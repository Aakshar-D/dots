import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Copy } from "lucide-react";
import { toast } from "sonner";

import { commands } from "@/api/commands";
import type { Dot } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidateDots, keys } from "@/lib/query";

function CopyButton({ what, text }: { what: string; text: string }) {
  return (
    <Button
      type="button"
      variant="outline"
      size="sm"
      aria-label={`Copy the ${what}`}
      onClick={() =>
        navigator.clipboard.writeText(text).then(() => toast.success(`Copied the ${what}`), notifyError)
      }
    >
      <Copy aria-hidden /> Copy
    </Button>
  );
}

/// How another program starts this dot: its trigger URL and bearer token.
export function WebhookPanel({ dot }: { dot: Dot }) {
  const qc = useQueryClient();
  const url = useQuery({
    queryKey: keys.webhookUrl(dot.id),
    queryFn: () => commands.webhookUrl(dot.id),
  });
  const regenerate = useMutation({
    mutationFn: () => commands.regenerateWebhookToken(dot.id),
    onSuccess: (token) => {
      invalidateDots(qc);
      qc.setQueryData(keys.dot(dot.id), { ...dot, webhook_token: token });
      toast.success("New token. The old one no longer works.");
    },
    onError: notifyError,
  });
  return (
    <div className="flex flex-col gap-3">
      <p className="text-xs text-muted-foreground">
        Send a POST request to the URL with the header{" "}
        <code>Authorization: Bearer &lt;token&gt;</code>. The request body becomes the run's
        payload.
      </p>
      <Field
        label="Webhook URL"
        htmlFor="webhook-url"
        error={url.isError ? errorMessage(url.error) : undefined}
      >
        <div className="flex gap-2">
          <Input id="webhook-url" readOnly className="font-mono" value={url.data ?? ""} />
          {url.data && <CopyButton what="webhook URL" text={url.data} />}
        </div>
      </Field>
      <Field label="Token" htmlFor="webhook-token">
        <div className="flex gap-2">
          <Input id="webhook-token" readOnly className="font-mono" value={dot.webhook_token} />
          <CopyButton what="token" text={dot.webhook_token} />
          <ConfirmButton
            title="Make a new token?"
            description="Anything that still sends the old token will be refused."
            confirmLabel="Make a new token"
            onConfirm={() => regenerate.mutate()}
            disabled={regenerate.isPending}
          >
            Regenerate
          </ConfirmButton>
        </div>
      </Field>
    </div>
  );
}
