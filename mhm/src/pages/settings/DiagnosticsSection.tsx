import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";

export default function DiagnosticsSection() {
  const [enabled, setEnabled] = useState(false);
  const [loading, setLoading] = useState(true);
  const [statusMessage, setStatusMessage] = useState<string | null>(null);

  useEffect(() => {
    invoke<boolean>("get_crash_reporting_preference")
      .then(setEnabled)
      .finally(() => setLoading(false));
  }, []);

  const handleToggle = async (nextValue: boolean) => {
    setEnabled(nextValue);
    setStatusMessage(null);

    try {
      await invoke("set_crash_reporting_preference", { enabled: nextValue });
      const message = nextValue
        ? "Severe crash reports are enabled"
        : "Severe crash reports are disabled";
      setStatusMessage(message);
      toast.success(message);
    } catch {
      setEnabled(!nextValue);
      setStatusMessage("Không thể cập nhật crash reporting");
      toast.error("Không thể cập nhật crash reporting");
    }
  };

  return (
    <div className="max-w-lg space-y-6">
      <div>
        <h3 className="mb-1 text-lg font-bold">Diagnostics</h3>
        <p className="text-sm text-brand-muted">
          Chỉ gửi báo cáo lỗi nghiêm trọng đã làm sạch dữ liệu nhạy cảm. Không theo dõi hành vi sử
          dụng.
        </p>
      </div>

      <label className="flex items-center justify-between rounded-xl border border-slate-200 p-4">
        <div className="pr-4">
          <p className="text-sm font-medium">Send crash reports</p>
          <p className="text-xs text-brand-muted">
            Báo cáo chỉ được gửi sau khi bạn đồng ý và không bao gồm dữ liệu khách hoặc session
            replay.
          </p>
        </div>
        <input
          type="checkbox"
          aria-label="Send crash reports"
          checked={enabled}
          disabled={loading}
          onChange={(event) => void handleToggle(event.target.checked)}
        />
      </label>

      {statusMessage && <p className="text-sm text-brand-text">{statusMessage}</p>}
    </div>
  );
}
