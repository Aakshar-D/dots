import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { SetupFlow } from "@/screens/setup/SetupFlow";

/// The home column's New dot (setup agent spec §4): the setup view in a dialog. Closing it while
/// the setup agent works cancels that request.
export function NewDotDialog({ onClose }: { onClose: () => void }) {
  return (
    <Dialog open onOpenChange={(next) => !next && onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>New dot</DialogTitle>
          <DialogDescription>
            Describe the task. Dots proposes the dot, and you check it before it is made.
          </DialogDescription>
        </DialogHeader>
        <SetupFlow title="What should the dot do?" onDone={onClose} />
      </DialogContent>
    </Dialog>
  );
}
