import { AlertTriangle, X } from "lucide-react";

import type { BackupReason } from "@/types";
import { cn } from "@/lib/utils";

export const BACKUP_FAILURE_FALLBACK_MESSAGE =
  "Không thể tạo bản sao lưu. Vui lòng kiểm tra dung lượng ổ đĩa hoặc thử lại.";

const BACKUP_REASON_LABELS: Record<BackupReason, string> = {
  settings: "Cài đặt",
  checkout: "Trả phòng",
  group_checkout: "Trả phòng nhóm",
  night_audit: "Night audit",
  app_exit: "Thoát ứng dụng",
  manual: "Thủ công",
  scheduled: "Tự động",
};

export type BackupFailureAlertState = {
  jobId: string;
  reason: BackupReason;
  message?: string | null;
};

type BackupFailureAlertProps = {
  failure: BackupFailureAlertState;
  onDismiss: (jobId: string) => void;
  className?: string;
};

function backupFailureMessage(message: string | null | undefined) {
  const trimmed = message?.trim();
  return trimmed ? trimmed : BACKUP_FAILURE_FALLBACK_MESSAGE;
}

export function BackupFailureAlert({ failure, onDismiss, className }: BackupFailureAlertProps) {
  const titleId = `backup-failure-alert-title-${failure.jobId}`;
  const messageId = `backup-failure-alert-message-${failure.jobId}`;
  const sourceId = `backup-failure-alert-source-${failure.jobId}`;

  return (
    <div
      role="alert"
      aria-labelledby={titleId}
      aria-describedby={`${messageId} ${sourceId}`}
      className={cn(
        "flex items-start gap-3 rounded-lg border border-rose-200 bg-white px-4 py-3 text-rose-800 shadow-soft",
        className,
      )}
    >
      <span className="mt-0.5 inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-rose-50 text-rose-600">
        <AlertTriangle aria-hidden="true" size={18} />
      </span>

      <div className="min-w-0 flex-1">
        <p id={titleId} className="text-sm font-semibold">
          Sao lưu thất bại
        </p>
        <p id={messageId} className="mt-1 text-sm text-rose-700">
          {backupFailureMessage(failure.message)}
        </p>
        <p id={sourceId} className="mt-1 text-xs font-medium text-rose-500">
          Nguồn: {BACKUP_REASON_LABELS[failure.reason]}
        </p>
      </div>

      <button
        type="button"
        aria-label="Đóng cảnh báo sao lưu"
        onClick={() => onDismiss(failure.jobId)}
        className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-rose-500 transition-colors hover:bg-rose-50 hover:text-rose-700"
      >
        <X aria-hidden="true" size={16} />
      </button>
    </div>
  );
}
