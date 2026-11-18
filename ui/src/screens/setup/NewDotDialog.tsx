import { useCallback, useState } from "react";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { DiscardConfirm, SetupFlow } from "@/screens/setup/SetupFlow";

/// The home column's New dot (setup agent spec §4): the setup view in a dialog. Closing it while
/// the setup agent works cancels that request; closing it with a proposed dot asks first; while
/// Run setup makes the dot it cannot be closed.
export function NewDotDialog({ onClose }: { onClose: () => void }) {
  const [proposed, setProposed] = useState(false);
  const [discarding, setDiscarding] = useState(false);
  const [applying, setApplying] = useState(false);
  const onProposal = useCallback((has: boolean) => setProposed(has), []);
  const onApplying = useCallback((on: boolean) => setApplying(on), []);
  return (
    <>
      <Dialog
        open
        onOpenChange={(next) => {
          // While Run setup makes the dot, the dialog stays: it closes itself once it is made.
          if (next || applying) return;
          if (proposed) setDiscarding(true);
          else onClose();
        }}
      >
        <DialogContent
          className="max-h-[90vh] overflow-y-auto sm:max-w-2xl"
          showCloseButton={!applying}
        >
          <DialogHeader>
            <DialogTitle>New sol</DialogTitle>
            <DialogDescription>
              Describe the task. Sage proposes the sol, and you check it before it is made.
            </DialogDescription>
          </DialogHeader>
          <SetupFlow
            title="What should the sol do?"
            onDone={onClose}
            onProposal={onProposal}
            onApplying={onApplying}
          />
        </DialogContent>
      </Dialog>
      <DiscardConfirm
        open={discarding}
        onOpenChange={setDiscarding}
        action="Close"
        onConfirm={onClose}
      />
    </>
  );
}
