import type { ComponentProps, ReactNode } from "react";

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { onSingleClick } from "@/lib/single-click";

/// A button for something that cannot be undone: it asks first.
export function ConfirmButton(props: {
  children: ReactNode;
  title: string;
  description: ReactNode;
  confirmLabel: string;
  onConfirm: () => void;
  disabled?: boolean;
  /// The trigger's accessible name, when its label alone does not say what it acts on.
  ariaLabel?: string;
  variant?: ComponentProps<typeof Button>["variant"];
  size?: ComponentProps<typeof Button>["size"];
}) {
  return (
    <AlertDialog>
      <AlertDialogTrigger asChild>
        <Button
          variant={props.variant ?? "outline"}
          size={props.size ?? "sm"}
          disabled={props.disabled}
          aria-label={props.ariaLabel}
        >
          {props.children}
        </Button>
      </AlertDialogTrigger>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{props.title}</AlertDialogTitle>
          <AlertDialogDescription>{props.description}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          {/* The dialog stays on the screen while it closes, so the second click of a
              double-click lands on this button again: only the first one confirms. */}
          <AlertDialogAction onClick={onSingleClick(props.onConfirm)}>
            {props.confirmLabel}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
