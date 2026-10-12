import type { ReactNode } from "react";

import { Label } from "@/components/ui/label";

/// A labelled control with its error, or its hint when there is none.
export function Field(props: {
  label: string;
  htmlFor: string;
  error?: string;
  hint?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={props.htmlFor}>{props.label}</Label>
      {props.children}
      {props.error ? (
        <p className="text-xs text-destructive">{props.error}</p>
      ) : (
        props.hint && <p className="text-xs text-muted-foreground">{props.hint}</p>
      )}
    </div>
  );
}
