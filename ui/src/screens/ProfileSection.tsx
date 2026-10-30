import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Copy } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";

import { commands } from "@/api/commands";
import { ErrorNote } from "@/components/ErrorNote";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

/// Settings → Profile (accounts spec §5): this install's Dots ID, which the person shares to be
/// added to an account, and the name and email others see for it.
export function ProfileSection() {
  const qc = useQueryClient();
  const identity = useQuery({ queryKey: keys.identity, queryFn: commands.identityGet });
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const saved = identity.data;
  // The form starts from what is saved, and follows it when a save answers.
  useEffect(() => {
    if (!saved) return;
    setName(saved.name);
    setEmail(saved.email);
  }, [saved]);
  const save = useMutation({
    mutationFn: () => commands.profileSet(name, email),
    onSuccess: (next) => {
      qc.setQueryData(keys.identity, next);
      invalidate(qc, keys.identity);
      toast.success("Saved your profile");
    },
  });
  return (
    <fieldset aria-label="Profile" className="flex max-w-3xl flex-col gap-3 rounded-lg border p-4">
      <legend className="px-1 text-sm font-semibold">Profile</legend>
      <p className="text-xs text-muted-foreground">
        Your Dots ID identifies this install. Share it with an account's admin to be added.
      </p>
      {identity.isError && <ErrorNote error={identity.error} />}
      {saved && (
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm text-muted-foreground">Dots ID</span>
          <code className="font-mono text-sm">{saved.fingerprint}</code>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() =>
              navigator.clipboard
                .writeText(saved.public_key)
                .then(() => toast.success("Copied your Dots ID"), notifyError)
            }
          >
            <Copy aria-hidden /> Copy my Dots ID
          </Button>
        </div>
      )}
      <form
        className="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <Field label="Name" htmlFor="profile-name">
          <Input id="profile-name" value={name} onChange={(e) => setName(e.target.value)} />
        </Field>
        <Field label="Email" htmlFor="profile-email">
          <Input
            id="profile-email"
            type="text"
            inputMode="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
          />
        </Field>
        {save.isError && <ErrorNote error={save.error} />}
        <Button type="submit" size="sm" className="self-start" disabled={!saved || save.isPending}>
          Save profile
        </Button>
      </form>
    </fieldset>
  );
}
