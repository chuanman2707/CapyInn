import { useState, useEffect, useRef, type ComponentType } from "react";
import { listen } from "@tauri-apps/api/event";
import { useHotelStore } from "./stores/useHotelStore";
import { useAuthStore } from "./stores/useAuthStore";
import Dashboard from "./pages/Dashboard";
import Rooms from "./pages/Rooms";
import Reservations from "./pages/Reservations";
import Guests from "./pages/Guests";
import Housekeeping from "./pages/Housekeeping";
import Analytics from "./pages/Analytics";
import Settings from "./pages/settings";
import NightAudit from "./pages/NightAudit";
import LoginScreen from "./pages/LoginScreen";
import OnboardingWizard from "./pages/onboarding";
import CheckinSheet from "./components/CheckinSheet";
import GroupCheckinSheet from "./components/GroupCheckinSheet";
import GroupManagement from "./pages/GroupManagement";
import AppLogo from "./components/AppLogo";
import CrashReportPrompt from "./components/CrashReportPrompt";
import AppUpdateBadge from "./components/AppUpdateBadge";
import AppUpdateRestartModal from "./components/AppUpdateRestartModal";
import { BackupStatusIndicator } from "./components/BackupStatusIndicator";
import { Home, Calendar, BedDouble, Users, Sparkles, BarChart3, Settings as SettingsIcon, ChevronsLeft, ChevronsRight, LogOut, Moon, UsersRound } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { AppUpdateProvider } from "@/contexts/AppUpdateContext";
import { APP_NAME } from "@/lib/appIdentity";
import { hasRemoteCrashReporting, submitCrashBundle } from "@/lib/crashReporting/sentry";
import type { CrashReportSummary } from "@/lib/crashReporting/types";
import { createDeferredCleanup } from "@/lib/deferredCleanup";
import { useAppUpdateController } from "@/hooks/useAppUpdateController";
import { Toaster, toast } from "sonner";
import { invoke } from "@tauri-apps/api/core";
import type { BackupIndicatorPhase, BackupStatusPayload, BootstrapStatus } from "@/types";

const NAV_MAIN = [
  { key: "dashboard" as const, label: "Dashboard", icon: Home },
  { key: "reservations" as const, label: "Reservations", icon: Calendar },
  { key: "rooms" as const, label: "Rooms", icon: BedDouble },
  { key: "guests" as const, label: "Guests", icon: Users },
  { key: "groups" as const, label: "Groups", icon: UsersRound },
];

const NAV_MANAGEMENT = [
  { key: "housekeeping" as const, label: "Housekeeping", icon: Sparkles },
  { key: "analytics" as const, label: "Analytics", icon: BarChart3 },
  { key: "audit" as const, label: "Night Audit", icon: Moon },
];

const NAV_SYSTEM = [
  { key: "settings" as const, label: "Settings", icon: SettingsIcon },
];

const PAGE_TITLES: Record<string, string> = {
  dashboard: "Overview",
  reservations: "Reservations",
  rooms: "Rooms",
  guests: "Guests",
  groups: "Group Booking",
  housekeeping: "Housekeeping",
  analytics: "Analytics",
  settings: "Settings",
  audit: "Night Audit",
};

type BackupUiState = {
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

export default function App() {
  const { activeTab, setTab, setCheckinOpen, setGroupCheckinOpen, checkinRoomId, fetchRooms, fetchStats } = useHotelStore();
  const { user, isAuthenticated, checkSession, logout, hydrateFromBootstrap } = useAuthStore();
  const [collapsed, setCollapsed] = useState(() => {
    return localStorage.getItem("sidebar-collapsed") === "true";
  });
  const [gatewayRunning, setGatewayRunning] = useState(false);
  const [bootstrap, setBootstrap] = useState<BootstrapStatus | null>(null);
  const [bootstrapLoading, setBootstrapLoading] = useState(true);
  const [backupUi, setBackupUi] = useState<BackupUiState>(INITIAL_BACKUP_UI);
  const [pendingCrashReport, setPendingCrashReport] = useState<CrashReportSummary | null>(null);
  const [crashPromptBusy, setCrashPromptBusy] = useState(false);
  const [crashExportPath, setCrashExportPath] = useState<string | null>(null);
  const didAutoCheckRef = useRef(false);
  const didCrashRecoveryCheckRef = useRef(false);
  const hideBackupRef = useRef<number | null>(null);
  const shellReady =
    !bootstrapLoading &&
    Boolean(bootstrap?.setup_completed) &&
    (!bootstrap?.app_lock_enabled || isAuthenticated);
  const appUpdate = useAppUpdateController({
    enabled: shellReady,
    supported: __UPDATER_ENABLED__,
    currentVersion: __APP_VERSION__,
  });

  useEffect(() => {
    invoke<BootstrapStatus>("get_bootstrap_status")
      .then((status) => {
        setBootstrap(status);
        if (status.current_user) {
          hydrateFromBootstrap(status.current_user);
        }
      })
      .finally(() => setBootstrapLoading(false));
  }, []);

  // Check session on mount for locked mode only
  useEffect(() => {
    if (!bootstrap?.app_lock_enabled) return;
    checkSession();
  }, [bootstrap?.app_lock_enabled]);

  useEffect(() => {
    if (!isAuthenticated) return;

    const cleanup = createDeferredCleanup(listen<{ entity: string }>("db-updated", () => {
      // Always refresh rooms and stats on any DB change
      fetchRooms();
      fetchStats();
    }));

    return cleanup;
  }, [isAuthenticated]);

  // Gateway status check
  useEffect(() => {
    if (!isAuthenticated) return;
    invoke<{ running: boolean }>("gateway_get_status")
      .then((s) => setGatewayRunning(s.running))
      .catch(() => setGatewayRunning(false));
  }, [isAuthenticated]);

  // MCP Gateway events: AI agent reservation notifications
  useEffect(() => {
    if (!isAuthenticated) return;
    const cleanup = createDeferredCleanup(listen<{ booking_id: string; room_id: string }>("mcp_reservation_created", (e) => {
      toast("🤖 AI Agent vừa tạo booking mới", {
        description: `Phòng ${e.payload.room_id} — ID: ${e.payload.booking_id}`,
      });
      fetchRooms();
      fetchStats();
    }));
    return cleanup;
  }, [isAuthenticated]);

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
    if (!shellReady || didAutoCheckRef.current) {
      return;
    }

    didAutoCheckRef.current = true;
    void appUpdate.checkForUpdates({ silent: true });
  }, [appUpdate, shellReady]);

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

  // Responsive: auto-collapse sidebar when window is narrow
  useEffect(() => {
    const handleResize = () => {
      if (window.innerWidth < 1200 && !collapsed) {
        setCollapsed(true);
        localStorage.setItem("sidebar-collapsed", "true");
      }
    };
    window.addEventListener("resize", handleResize);
    handleResize();
    return () => window.removeEventListener("resize", handleResize);
  }, [collapsed]);

  const toggleCollapse = () => {
    const next = !collapsed;
    setCollapsed(next);
    localStorage.setItem("sidebar-collapsed", String(next));
  };

  if (bootstrapLoading) {
    return (
      <div className="h-screen w-screen grid place-items-center bg-brand-bg text-sm text-brand-muted">
        Loading...
      </div>
    );
  }

  if (bootstrap && !bootstrap.setup_completed) {
    return (
      <>
        <OnboardingWizard onCompleted={(status) => {
          setBootstrap(status);
          hydrateFromBootstrap(status.current_user);
        }} />
        <Toaster position="bottom-right" toastOptions={{ className: "rounded-xl shadow-float font-sans" }} />
      </>
    );
  }

  // If app lock is enabled and user is not authenticated, show login screen
  if (bootstrap?.app_lock_enabled && !isAuthenticated) {
    return (
      <>
        <LoginScreen />
        <Toaster position="bottom-right" toastOptions={{ className: "rounded-xl shadow-float font-sans" }} />
      </>
    );
  }

  const today = new Date().toLocaleDateString("vi-VN", {
    weekday: "long",
    day: "numeric",
    month: "long",
    year: "numeric"
  });

  const renderNavItem = (item: { key: string; label: string; icon: ComponentType<{ size?: number; className?: string }> }) => {
    const isActive = activeTab === item.key;
    const Icon = item.icon;
    return (
      <Button
        key={item.key}
        variant={isActive ? "secondary" : "ghost"}
        className={`justify-start rounded-xl font-medium ${collapsed ? "px-3" : ""} ${isActive ? 'bg-brand-primary/10 text-brand-primary hover:bg-brand-primary/20' : 'text-brand-muted hover:text-brand-text'}`}
        size="lg"
        onClick={() => setTab(item.key as any)}
        title={collapsed ? item.label : undefined}
      >
        <Icon className={collapsed ? "" : "mr-3"} size={20} />
        {!collapsed && item.label}
      </Button>
    );
  };

  const handleUpdateBadgeClick = async () => {
    if (appUpdate.phase === "available") {
      await appUpdate.downloadUpdate();
    }
  };

  const handleSendCrashReport = async () => {
    if (!pendingCrashReport) {
      return;
    }

    setCrashPromptBusy(true);
    try {
      await submitCrashBundle(pendingCrashReport);
    } catch {
      await invoke("mark_crash_report_send_failed", {
        bundle_id: pendingCrashReport.bundle_id,
      });
      toast.error("Gửi crash report thất bại");
      return;
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
  };

  const handleDismissCrashReport = async () => {
    if (!pendingCrashReport) {
      return;
    }

    setCrashPromptBusy(true);
    try {
      await invoke("mark_crash_report_dismissed", {
        bundle_id: pendingCrashReport.bundle_id,
      });
      setPendingCrashReport(null);
      setCrashExportPath(null);
    } finally {
      setCrashPromptBusy(false);
    }
  };

  const handleExportCrashReport = async () => {
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
  };

  return (
    <AppUpdateProvider value={appUpdate}>
      <div className="flex h-screen w-screen bg-brand-bg font-sans text-brand-text overflow-hidden select-none">
        <BackupStatusIndicator
          visible={backupUi.visible}
          phase={backupUi.phase}
          message={backupUi.message}
        />
        <AppUpdateRestartModal
          open={appUpdate.restartPromptOpen}
          version={appUpdate.availableVersion ?? appUpdate.currentVersion}
          onConfirm={appUpdate.confirmInstall}
          onLater={appUpdate.dismissRestartPrompt}
        />
        {pendingCrashReport && (
          <CrashReportPrompt
            report={pendingCrashReport}
            remoteEnabled={hasRemoteCrashReporting()}
            busy={crashPromptBusy}
            exportPath={crashExportPath}
            onSend={handleSendCrashReport}
            onDismiss={handleDismissCrashReport}
            onExport={handleExportCrashReport}
          />
        )}

        {/* SIDEBAR */}
        <aside className={`${collapsed ? "w-[72px]" : "w-[260px]"} bg-white border-r border-slate-100 flex flex-col z-20 shrink-0 transition-all duration-300`}>

        {/* Logo */}
        <div className={`${collapsed ? "px-4 py-6" : "p-6"} mb-4 flex justify-center`}>
          <AppLogo className={collapsed ? "h-10 w-10 shrink-0" : "h-14 w-14 shrink-0"} />
        </div>

        {/* Navigation */}
        <nav className={`flex flex-col gap-1 ${collapsed ? "px-2" : "px-4"} flex-1 min-h-0 overflow-y-auto`}>
          {/* Main */}
          {!collapsed && <span className="text-[10px] font-bold text-brand-muted uppercase tracking-widest mb-2 ml-3">Main</span>}
          {NAV_MAIN.map(renderNavItem)}

          {/* Management */}
          <div className="my-3 border-t border-slate-100" />
          {!collapsed && <span className="text-[10px] font-bold text-brand-muted uppercase tracking-widest mb-2 ml-3">Management</span>}
          {NAV_MANAGEMENT.map(renderNavItem)}

          {/* System */}
          <div className="my-3 border-t border-slate-100" />
          {!collapsed && <span className="text-[10px] font-bold text-brand-muted uppercase tracking-widest mb-2 ml-3">System</span>}
          {NAV_SYSTEM.map(renderNavItem)}
        </nav>

        {/* User info + Collapse */}
        <div className={`${collapsed ? "px-2" : "px-4"} pb-4 space-y-2 shrink-0`}>
          {/* User info */}
          {user && !collapsed && (
            <div className="flex items-center gap-2 px-3 py-2 bg-slate-50 rounded-xl">
              <div className="w-7 h-7 rounded-lg bg-brand-primary/10 text-brand-primary flex items-center justify-center text-xs font-bold">
                {user.name.charAt(0).toUpperCase()}
              </div>
              <div className="flex-1 min-w-0">
                <p className="text-xs font-semibold truncate">{user.name}</p>
                <p className="text-[10px] text-brand-muted capitalize">{user.role}</p>
              </div>
              <button
                onClick={logout}
                className="text-brand-muted hover:text-red-500 transition-colors cursor-pointer"
                title="Đăng xuất"
              >
                <LogOut size={14} />
              </button>
            </div>
          )}
          {user && collapsed && (
            <button
              onClick={logout}
              className="w-full flex justify-center py-2 text-brand-muted hover:text-red-500 transition-colors cursor-pointer"
              title="Đăng xuất"
            >
              <LogOut size={16} />
            </button>
          )}
          <Button
            variant="ghost"
            className="w-full justify-center rounded-xl opacity-40 hover:opacity-100"
            size="sm"
            onClick={toggleCollapse}
          >
            {collapsed ? <ChevronsRight size={16} /> : <><ChevronsLeft size={16} className="mr-2" /> Thu gọn</>}
          </Button>
        </div>
        </aside>

        {/* MAIN CONTENT */}
        <main className="flex-1 flex flex-col h-full relative min-w-0">

        {/* HEADER */}
        <header className="h-[88px] flex items-center justify-between px-10 bg-brand-bg/80 backdrop-blur-md sticky top-0 z-10 data-tauri-drag-region shrink-0">
          <div className="pointer-events-none">
            <h1 className="text-2xl font-bold tracking-tight">
              {PAGE_TITLES[activeTab] || APP_NAME}
            </h1>
            <p className="text-sm text-brand-muted">{today}</p>
          </div>

          <div className="flex items-center gap-4 pointer-events-auto">
            <AppUpdateBadge
              phase={appUpdate.phase}
              onCheckOrDownload={handleUpdateBadgeClick}
              onRestart={appUpdate.openRestartPrompt}
            />
            {user && (
              <Badge className={`${user.role === 'admin' ? 'bg-amber-50 text-amber-700' : 'bg-blue-50 text-blue-700'} border-0 rounded-full py-1.5 px-3 uppercase tracking-wider text-[10px] font-bold`}>
                {user.role === 'admin' ? '👑 Admin' : '🏨 Lễ tân'}
              </Badge>
            )}
            <Badge className={`${gatewayRunning ? 'bg-emerald-50 text-emerald-700' : 'bg-red-50 text-red-500'} border-0 rounded-full py-1.5 px-3 uppercase tracking-wider text-[10px] font-bold cursor-pointer`} onClick={() => setTab('settings' as any)}>
              {gatewayRunning ? '● MCP Gateway' : '○ Gateway Off'}
            </Badge>
            <Badge className="bg-green-50 text-green-700 border-0 rounded-full py-1.5 px-3 uppercase tracking-wider text-[10px] font-bold">
              ● Scanner Ready
            </Badge>
            <Button onClick={() => setGroupCheckinOpen(true)} className="rounded-xl bg-brand-primary text-white shadow-soft hover:shadow-float transition-all px-5 py-5">
              <UsersRound size={16} className="mr-1.5" /> Đoàn mới
            </Button>
            <Button onClick={() => setCheckinOpen(true)} className="rounded-xl bg-brand-primary text-white shadow-soft hover:shadow-float transition-all px-6 py-5">
              + Khách mới
            </Button>
          </div>
        </header>

        {/* CONTENT AREA */}
        <div className="flex-1 overflow-y-auto px-10 pb-10">
          <div className="animate-fade-up">
            {activeTab === "dashboard" && <Dashboard />}
            {activeTab === "rooms" && <Rooms />}
            {activeTab === "reservations" && <Reservations />}
            {activeTab === "guests" && <Guests />}
            {activeTab === "groups" && <GroupManagement />}
            {activeTab === "housekeeping" && <Housekeeping />}
            {activeTab === "analytics" && <Analytics />}
            {activeTab === "audit" && <NightAudit />}
            {activeTab === "settings" && <Settings />}
          </div>
        </div>

        </main>

        <CheckinSheet preSelectedRoomId={checkinRoomId ?? undefined} />
        <GroupCheckinSheet />
        <Toaster position="bottom-right" toastOptions={{ className: "rounded-xl shadow-float font-sans" }} />
      </div>
    </AppUpdateProvider>
  );
}
