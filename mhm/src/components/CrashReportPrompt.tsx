import Modal from "@/components/ui/Modal";
import { Button } from "@/components/ui/button";
import type { CrashReportSummary } from "@/lib/crashReporting/types";

interface CrashReportPromptProps {
  report: CrashReportSummary;
  remoteEnabled: boolean;
  busy: boolean;
  exportPath: string | null;
  onSend: () => Promise<void>;
  onDismiss: () => Promise<void>;
  onExport: () => Promise<void>;
}

export default function CrashReportPrompt({
  report,
  remoteEnabled,
  busy,
  exportPath,
  onSend,
  onDismiss,
  onExport,
}: CrashReportPromptProps) {
  return (
    <Modal title="App encountered a serious error">
      <div className="space-y-4 text-sm text-brand-muted">
        <p>
          CapyInn gặp lỗi nghiêm trọng trong phiên trước. Báo cáo này đã được làm sạch dữ liệu
          nhạy cảm và không chứa tracking hành vi sử dụng.
        </p>
        <p className="text-xs">Crash type: {report.crash_type}</p>
        <p className="text-xs">Message: {report.message}</p>
        {!remoteEnabled && (
          <p className="text-xs text-amber-700">
            Remote reporting chưa được cấu hình trong build này. Bạn vẫn có thể export report cục
            bộ.
          </p>
        )}
        {exportPath && <p className="text-xs text-emerald-600">{exportPath}</p>}
        <div className="flex justify-end gap-2">
          <Button variant="outline" disabled={busy} onClick={() => void onExport()}>
            Export report
          </Button>
          <Button variant="outline" disabled={busy} onClick={() => void onDismiss()}>
            Don't send
          </Button>
          <Button disabled={busy || !remoteEnabled} onClick={() => void onSend()}>
            Send report
          </Button>
        </div>
      </div>
    </Modal>
  );
}
