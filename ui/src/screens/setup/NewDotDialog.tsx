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
/// the setup agent works cancels that request; closing it with a proposed dot asks first.
export function NewDotDialog({ onClose }: { onClose: () => void }) {
  const [proposed, setProposed] = useState(false);
  const [discarding, setDiscarding] = useState(false);
  const onProposal = useCallback((has: boolean) => setProposed(has), []);
  return (
    <>
      <Dialog
        open
        onOpenChange={(next) => {
          if (next) return;
          if (proposed) setDiscarding(true);
          else onClose();
        }}
      >
        <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>New dot</DialogTitle>
            <DialogDescription>
              Describe the task. Dots proposes the dot, and you check it before it is made.
            </DialogDescription>
          </DialogHeader>
          <SetupFlow title="What should the dot do?" onDone={onClose} onProposal={onProposal} />
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
