import { Button } from "@/components/ui/button";
import { useAppUpdate } from "@/contexts/AppUpdateContext";

export default function SoftwareUpdateSection() {
  const update = useAppUpdate();
  const isBusy =
    update.phase === "checking" ||
    update.phase === "downloading" ||
    update.phase === "installing";

  return (
    <div className="space-y-6 max-w-lg">
      <div>
        <h3 className="text-lg font-bold mb-1">Software Update</h3>
        <p className="text-sm text-brand-muted">Kiểm tra và áp dụng bản CapyInn mới.</p>
      </div>

      <div className="space-y-3 p-4 bg-slate-50 rounded-xl">
        <div>
          <p className="text-sm font-medium">Current version</p>
          <p className="text-xs text-brand-muted">{update.currentVersion}</p>
        </div>

        {update.availableVersion && (
          <div>
            <p className="text-sm font-medium">Available version</p>
            <p className="text-xs text-brand-muted">{update.availableVersion}</p>
          </div>
        )}
      </div>

      <div className="flex gap-3">
        <Button
          variant="outline"
          onClick={() => void update.checkForUpdates({ silent: false })}
          disabled={isBusy}
        >
          Check for updates
        </Button>

        {update.phase === "available" && (
          <Button onClick={() => void update.downloadUpdate()}>Update</Button>
        )}

        {update.phase === "downloaded" && (
          <Button onClick={update.openRestartPrompt}>Restart to update</Button>
        )}
      </div>

      {update.errorMessage && <p className="text-xs text-rose-600">{update.errorMessage}</p>}

      {!update.availableVersion && update.phase === "idle" && (
        <p className="text-xs text-emerald-600">You are on the latest version.</p>
      )}
    </div>
  );
}
