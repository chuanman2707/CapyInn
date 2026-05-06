import { useEffect, useState } from "react";
import { toast } from "sonner";

import { invokeCommand, invokeWriteCommand } from "@/lib/invokeCommand";

export default function CeoAgentSection() {
  const [enabled, setEnabled] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [statusMessage, setStatusMessage] = useState<string | null>(null);

  useEffect(() => {
    invokeCommand<boolean>("get_ceo_cloud_data_opt_in")
      .then((value) => {
        setEnabled(value);
        setLoadError(null);
      })
      .catch(() => {
        setLoadError("Unable to load CEO cloud-data opt-in");
      })
      .finally(() => setLoading(false));
  }, []);

  const handleToggle = async (nextValue: boolean) => {
    const previous = enabled;
    setSaving(true);
    setEnabled(nextValue);
    setStatusMessage(null);

    try {
      await invokeWriteCommand("set_ceo_cloud_data_opt_in", { enabled: nextValue });
      const message = nextValue
        ? "CEO cloud-data opt-in enabled"
        : "CEO cloud-data opt-in revoked";
      setStatusMessage(message);
      toast.success(message);
    } catch {
      setEnabled(previous);
      setStatusMessage("Unable to update CEO cloud-data opt-in");
      toast.error("Unable to update CEO cloud-data opt-in");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="max-w-lg space-y-6">
      <div>
        <h3 className="mb-1 text-lg font-bold">CEO Agent</h3>
        <p className="text-sm text-brand-muted">
          Opt-in is required before future CEO-sensitive cloud LLM processing.
        </p>
      </div>

      <label className="flex items-center justify-between rounded-xl border border-slate-200 p-4">
        <div className="pr-4 space-y-1">
          <p className="text-sm font-medium">Allow CEO cloud-data processing</p>
          <div className="text-xs text-brand-muted space-y-1">
            <p>Opt-in is revocable.</p>
            <p>Revoking blocks cloud calls containing CEO-sensitive PMS data.</p>
            <p>
              Raw prompts, raw responses, raw tool outputs, and raw provider errors are not stored.
            </p>
            <p>Runtime remains disabled in this build.</p>
          </div>
        </div>
        <input
          type="checkbox"
          aria-label="Allow CEO cloud-data processing"
          checked={enabled}
          disabled={loading || saving || Boolean(loadError)}
          onChange={(event) => void handleToggle(event.target.checked)}
        />
      </label>

      {loadError && <p className="text-sm text-red-500">{loadError}</p>}
      {statusMessage && <p className="text-sm text-brand-text">{statusMessage}</p>}
    </div>
  );
}
