import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";

import { type BackupFailureAlertState } from "@/components/BackupFailureAlert";
import { useBootstrapState } from "@/app/BootstrapState";
import { hasRemoteCrashReporting, submitCrashBundle } from "@/lib/crashReporting/sentry";
import type { CrashReportSummary } from "@/lib/crashReporting/types";
import { createDeferredCleanup } from "@/lib/deferredCleanup";
import { useExperimentalRuntimeStatus } from "@/lib/experimentalProfile";
import { useAuthStore } from "@/stores/useAuthStore";
import { useHotelStore } from "@/stores/useHotelStore";
import type { BackupIndicatorPhase, BackupStatusPayload } from "@/types";

export type BackupUiState = {
  visible: boolean;
  phase: BackupIndicatorPhase;
  message: string;
  pendingJobs: number;
};

const INITIAL_BACKUP_UI: BackupUiState = {
  visible: false,
  phase: "saved",
  message: "",
  pendingJobs: 0,
};

type RuntimeStateValue = {
  backupUi: BackupUiState;
  visibleBackupFailure: BackupFailureAlertState | null;
  onDismissBackupFailure: (jobId: string) => void;
  pendingCrashReport: CrashReportSummary | null;
  crashPromptBusy: boolean;
  crashExportPath: string | null;
  onSendCrashReport: () => Promise<void>;
  onDismissCrashReport: () => Promise<void>;
  onExportCrashReport: () => Promise<void>;
  gatewayRunning: boolean;
  gatewayRuntimeEnabled: boolean;
  remoteCrashReportingEnabled: boolean;
};

const RuntimeStateContext = createContext<RuntimeStateValue | null>(null);

export function RuntimeStateProvider({ children }: { children: ReactNode }) {
  const { shellReady } = useBootstrapState();
  const isAuthenticated = useAuthStore((state) => state.isAuthenticated);
  const fetchRooms = useHotelStore((state) => state.fetchRooms);
  const fetchStats = useHotelStore((state) => state.fetchStats);
  const [gatewayRunning, setGatewayRunning] = useState(false);
  const [backupUi, setBackupUi] = useState<BackupUiState>(INITIAL_BACKUP_UI);
  const [backupFailure, setBackupFailure] = useState<BackupFailureAlertState | null>(null);
  const [dismissedBackupFailureJobId, setDismissedBackupFailureJobId] = useState<string | null>(
    null,
  );
  const [pendingCrashReport, setPendingCrashReport] = useState<CrashReportSummary | null>(null);
  const [crashPromptBusy, setCrashPromptBusy] = useState(false);
  const [crashExportPath, setCrashExportPath] = useState<string | null>(null);
  const didCrashRecoveryCheckRef = useRef(false);
  const hideBackupRef = useRef<number | null>(null);
  const remoteCrashReportingEnabled = hasRemoteCrashReporting();
  const experimentalRuntime = useExperimentalRuntimeStatus(isAuthenticated);
  const gatewayRuntimeEnabled = experimentalRuntime.gatewayRuntimeEnabled;

  useEffect(() => {
    if (!isAuthenticated) return;

    const cleanup = createDeferredCleanup(
      listen<{ entity: string }>("db-updated", () => {
        fetchRooms();
        fetchStats();
      }),
    );

    return cleanup;
  }, [fetchRooms, fetchStats, isAuthenticated]);

  useEffect(() => {
    if (!isAuthenticated || !gatewayRuntimeEnabled) {
      setGatewayRunning(false);
      return;
    }

    let cancelled = false;

    invoke<{ running: boolean }>("gateway_get_status")
      .then((status) => {
        if (!cancelled) {
          setGatewayRunning(status.running);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setGatewayRunning(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [gatewayRuntimeEnabled, isAuthenticated]);

  useEffect(() => {
    if (!isAuthenticated || !gatewayRuntimeEnabled) return;

    const cleanup = createDeferredCleanup(
      listen<{ booking_id: string; room_id: string }>("mcp_reservation_created", (event) => {
        toast("🤖 AI Agent vừa tạo booking mới", {
          description: `Phòng ${event.payload.room_id} — ID: ${event.payload.booking_id}`,
        });
        fetchRooms();
        fetchStats();
      }),
    );

    return cleanup;
  }, [fetchRooms, fetchStats, gatewayRuntimeEnabled, isAuthenticated]);

  useEffect(() => {
    const cleanup = createDeferredCleanup(
      listen<BackupStatusPayload>("backup-status", ({ payload }) => {
        if (hideBackupRef.current !== null) {
          window.clearTimeout(hideBackupRef.current);
          hideBackupRef.current = null;
        }

        if (payload.state === "started") {
          setBackupUi({
            visible: true,
            phase: "saving",
            message: "Đang sao lưu dữ liệu...",
            pendingJobs: payload.pending_jobs,
          });
          return;
        }

        if (payload.state === "failed") {
          toast.error(payload.message ?? "Sao lưu dữ liệu thất bại");
          setDismissedBackupFailureJobId((current) =>
            current === payload.job_id ? current : null,
          );
          setBackupFailure({
            jobId: payload.job_id,
            reason: payload.reason,
            message: payload.message,
          });
          setBackupUi({
            visible: true,
            phase: "failed",
            message: "Sao lưu thất bại",
            pendingJobs: payload.pending_jobs,
          });
          return;
        }

        if (payload.pending_jobs > 0) {
          setBackupUi({
            visible: true,
            phase: "saving",
            message: "Đang sao lưu dữ liệu...",
            pendingJobs: payload.pending_jobs,
          });
          return;
        }

        setBackupFailure(null);
        setDismissedBackupFailureJobId(null);
        setBackupUi({
          visible: true,
          phase: "saved",
          message: "Đã sao lưu",
          pendingJobs: 0,
        });

        hideBackupRef.current = window.setTimeout(() => {
          setBackupUi(INITIAL_BACKUP_UI);
          hideBackupRef.current = null;
        }, 1800);
      }),
    );

    return () => {
      if (hideBackupRef.current !== null) {
        window.clearTimeout(hideBackupRef.current);
        hideBackupRef.current = null;
      }
      cleanup();
    };
  }, []);

  useEffect(() => {
    if (!shellReady || didCrashRecoveryCheckRef.current) {
      return;
    }

    didCrashRecoveryCheckRef.current = true;

    void invoke<boolean>("get_crash_reporting_preference")
      .then(async (enabled) => {
        const pending = await invoke<CrashReportSummary | null>("get_pending_crash_report");
        if (!pending) {
          return;
        }

        setCrashExportPath(null);

        if (enabled && hasRemoteCrashReporting()) {
          try {
            await submitCrashBundle(pending);
          } catch {
            await invoke("mark_crash_report_send_failed", { bundle_id: pending.bundle_id });
            setPendingCrashReport(pending);
            return;
          }

          try {
            await invoke("mark_crash_report_submitted", { bundle_id: pending.bundle_id });
            return;
          } catch {
            toast.error("Crash report đã gửi nhưng không thể dọn bundle cục bộ");
            setPendingCrashReport(null);
          }
        }

        setPendingCrashReport(pending);
      })
      .catch(() => {
        // Diagnostics lookups must never block the shell.
      });
  }, [shellReady]);

  const onSendCrashReport = useCallback(async () => {
    if (!pendingCrashReport) {
      return;
    }

    setCrashPromptBusy(true);
    let sendSucceeded = false;
    try {
      await submitCrashBundle(pendingCrashReport);
      sendSucceeded = true;
    } catch {
      await invoke("mark_crash_report_send_failed", {
        bundle_id: pendingCrashReport.bundle_id,
      });
      toast.error("Gửi crash report thất bại");
      return;
    } finally {
      if (!sendSucceeded) {
        setCrashPromptBusy(false);
      }
    }

    try {
      await invoke("set_crash_reporting_preference", { enabled: true });
      await invoke("mark_crash_report_submitted", {
        bundle_id: pendingCrashReport.bundle_id,
      });
      setPendingCrashReport(null);
      setCrashExportPath(null);
    } catch {
      toast.error("Crash report đã gửi nhưng không thể hoàn tất cleanup cục bộ");
    } finally {
      setCrashPromptBusy(false);
    }
  }, [pendingCrashReport]);

  const onDismissCrashReport = useCallback(async () => {
    if (!pendingCrashReport) {
      return;
    }

    const report = pendingCrashReport;
    setCrashPromptBusy(true);
    try {
      await invoke("mark_crash_report_dismissed", {
        bundle_id: report.bundle_id,
      });
    } catch {
      toast.error("Không thể dọn crash report cục bộ. Prompt sẽ được ẩn cho phiên này.");
    } finally {
      setPendingCrashReport(null);
      setCrashExportPath(null);
      setCrashPromptBusy(false);
    }
  }, [pendingCrashReport]);

  const onExportCrashReport = useCallback(async () => {
    if (!pendingCrashReport) {
      return;
    }

    setCrashPromptBusy(true);
    try {
      const path = await invoke<string>("export_crash_report", {
        bundle_id: pendingCrashReport.bundle_id,
      });
      setCrashExportPath(path);
    } finally {
      setCrashPromptBusy(false);
    }
  }, [pendingCrashReport]);

  const visibleBackupFailure =
    backupFailure && backupFailure.jobId !== dismissedBackupFailureJobId ? backupFailure : null;

  const onDismissBackupFailure = useCallback((jobId: string) => {
    setDismissedBackupFailureJobId(jobId);
  }, []);

  const value = useMemo(
    () => ({
      backupUi,
      visibleBackupFailure,
      onDismissBackupFailure,
      pendingCrashReport,
      crashPromptBusy,
      crashExportPath,
      onSendCrashReport,
      onDismissCrashReport,
      onExportCrashReport,
      gatewayRunning,
      gatewayRuntimeEnabled,
      remoteCrashReportingEnabled,
    }),
    [
      backupUi,
      visibleBackupFailure,
      onDismissBackupFailure,
      pendingCrashReport,
      crashPromptBusy,
      crashExportPath,
      onSendCrashReport,
      onDismissCrashReport,
      onExportCrashReport,
      gatewayRunning,
      gatewayRuntimeEnabled,
      remoteCrashReportingEnabled,
    ],
  );

  return <RuntimeStateContext.Provider value={value}>{children}</RuntimeStateContext.Provider>;
}

export function useRuntimeState() {
  const value = useContext(RuntimeStateContext);

  if (!value) {
    throw new Error("useRuntimeState must be used inside RuntimeStateProvider");
  }

  return value;
}
