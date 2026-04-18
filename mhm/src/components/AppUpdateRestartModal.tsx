import Modal from "@/components/ui/Modal";
import { Button } from "@/components/ui/button";

interface AppUpdateRestartModalProps {
  open: boolean;
  version: string;
  onConfirm: () => void | Promise<void>;
  onLater: () => void;
}

export default function AppUpdateRestartModal({
  open,
  version,
  onConfirm,
  onLater,
}: AppUpdateRestartModalProps) {
  if (!open) {
    return null;
  }

  return (
    <Modal title="Update ready">
      <div className="space-y-4">
        <p className="text-sm text-slate-600">
          Version {version} is ready to install. Restart the app when you are ready to apply it.
        </p>
        <div className="flex justify-end gap-2">
          <Button variant="outline" onClick={onLater}>Later</Button>
          <Button onClick={() => void onConfirm()}>Restart to update</Button>
        </div>
      </div>
    </Modal>
  );
}
