import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
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

/// How another program starts this dot: its trigger URL and bearer token. Dots keeps only the
/// token's hash, so a token is shown once, when it is made.
export function WebhookPanel({ dot }: { dot: Dot }) {
  const qc = useQueryClient();
  const [shown, setShown] = useState<string | null>(null);
  const url = useQuery({
    queryKey: keys.webhookUrl(dot.id),
    queryFn: () => commands.webhookUrl(dot.id),
  });
  const regenerate = useMutation({
    mutationFn: () => commands.regenerateWebhookToken(dot.id),
    onSuccess: (token) => {
      setShown(token);
      invalidateDots(qc);
      qc.setQueryData(keys.dot(dot.id), { ...dot, has_webhook_token: true });
      toast.success(
        dot.has_webhook_token ? "New token. The old one no longer works." : "Token made.",
      );
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
      {shown !== null ? (
        <Field
          label="Token"
          htmlFor="webhook-token"
          hint="Copy it now: you won't see this again."
        >
          <div className="flex gap-2">
            <Input id="webhook-token" readOnly className="font-mono" value={shown} />
            <CopyButton what="token" text={shown} />
          </div>
        </Field>
      ) : (
        <p className="text-sm text-muted-foreground">
          {dot.has_webhook_token
            ? "This dot has a token. It was shown once, when it was made."
            : "This dot has no token yet, so its webhook refuses every request."}
        </p>
      )}
      <div>
        {dot.has_webhook_token ? (
          <ConfirmButton
            title="Make a new token?"
            description="Anything that still sends the old token will be refused."
            confirmLabel="Make a new token"
            onConfirm={() => regenerate.mutate()}
            disabled={regenerate.isPending}
          >
            Regenerate
          </ConfirmButton>
        ) : (
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => regenerate.mutate()}
            disabled={regenerate.isPending}
          >
            Make a token
          </Button>
        )}
      </div>
    </div>
  );
}
