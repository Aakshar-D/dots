import { useQuery } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import { Field } from "@/components/Field";
import { Input } from "@/components/ui/input";

/// Browser use in Settings (browser spec section 7): whether Microsoft Edge and Node.js were
/// found, and the Edge path. A new path is confirmed in a system dialog when it is saved;
/// `declined` says that the person said no.
export function BrowserSettings(props: {
  edgePath: string;
  onEdgePath: (path: string) => void;
  declined: boolean;
}) {
  const status = useQuery({ queryKey: ["browser-status"], queryFn: commands.browserStatus });
  const edge = status.data?.edge ?? null;
  const npx = status.data?.npx ?? null;
  return (
    <>
      {status.data && (
        <ul aria-label="What browser use needs" className="flex flex-col gap-1 text-xs">
          <li>
            <span>{edge ? "Microsoft Edge found" : "Microsoft Edge not found"}</span>
            {edge && <span className="ml-2 font-mono text-muted-foreground">{edge}</span>}
          </li>
          <li>
            <span>{npx ? "Node.js found" : "Node.js not found"}</span>
            {npx && <span className="ml-2 font-mono text-muted-foreground">{npx}</span>}
          </li>
        </ul>
      )}
      <Field
        label="Edge path"
        htmlFor="settings-edge-path"
        error={props.declined ? "Not saved: you declined the change of the Edge program." : undefined}
        hint="Leave empty to use Microsoft Edge where it is installed. A new program must be confirmed in a system dialog."
      >
        <Input
          id="settings-edge-path"
          className="font-mono"
          value={props.edgePath}
          onChange={(e) => props.onEdgePath(e.target.value)}
        />
      </Field>
    </>
  );
}
