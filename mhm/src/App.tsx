import { useState, useEffect, type ComponentType } from "react";
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
import CheckinSheet from "./components/CheckinSheet";
import GroupCheckinSheet from "./components/GroupCheckinSheet";
import GroupManagement from "./pages/GroupManagement";
import AppLogo from "./components/AppLogo";
import CrashReportPrompt from "./components/CrashReportPrompt";
import AppUpdateBadge from "./components/AppUpdateBadge";
import AppUpdateRestartModal from "./components/AppUpdateRestartModal";
import { BackupStatusIndicator } from "./components/BackupStatusIndicator";
import { BackupFailureAlert } from "./components/BackupFailureAlert";
import { Home, Calendar, BedDouble, Users, Sparkles, BarChart3, Settings as SettingsIcon, ChevronsLeft, ChevronsRight, LogOut, Moon, UsersRound } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { AppToaster } from "@/app/AppToaster";
import { AppUpdateRuntime } from "@/app/AppUpdateRuntime";
import { AuthGate } from "@/app/AuthGate";
import { BootstrapGate } from "@/app/BootstrapGate";
import { BootstrapStateProvider } from "@/app/BootstrapState";
import { RuntimeStateProvider, useRuntimeState } from "@/app/RuntimeStateProvider";
import { isExperimentalGatewayUiEnabled } from "@/app/runtimeProfile";
import { useAppUpdate } from "@/contexts/AppUpdateContext";
import { APP_NAME } from "@/lib/appIdentity";

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

export default function App() {
  const experimentalGatewayUi = isExperimentalGatewayUiEnabled();

  return (
    <BootstrapStateProvider>
      <AppUpdateRuntime>
        <RuntimeStateProvider experimentalGatewayUi={experimentalGatewayUi}>
          <BootstrapGate>
            {({ bootstrap }) => (
              <AuthGate bootstrap={bootstrap}>
                <CurrentAppShell experimentalGatewayUi={experimentalGatewayUi} />
              </AuthGate>
            )}
          </BootstrapGate>
        </RuntimeStateProvider>
      </AppUpdateRuntime>
    </BootstrapStateProvider>
  );
}

function CurrentAppShell({ experimentalGatewayUi }: { experimentalGatewayUi: boolean }) {
  const appUpdate = useAppUpdate();
  const { activeTab, setTab, setCheckinOpen, setGroupCheckinOpen, checkinRoomId } = useHotelStore();
  const { user, logout } = useAuthStore();
  const {
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
    remoteCrashReportingEnabled,
  } = useRuntimeState();
  const [collapsed, setCollapsed] = useState(() => {
    return localStorage.getItem("sidebar-collapsed") === "true";
  });

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

  return (
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
            remoteEnabled={remoteCrashReportingEnabled}
            busy={crashPromptBusy}
            exportPath={crashExportPath}
            onSend={onSendCrashReport}
            onDismiss={onDismissCrashReport}
            onExport={onExportCrashReport}
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
            {experimentalGatewayUi && (
              <Badge className={`${gatewayRunning ? 'bg-emerald-50 text-emerald-700' : 'bg-red-50 text-red-500'} border-0 rounded-full py-1.5 px-3 uppercase tracking-wider text-[10px] font-bold cursor-pointer`} onClick={() => setTab('settings' as any)}>
                {gatewayRunning ? '● MCP Gateway' : '○ Gateway Off'}
              </Badge>
            )}
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

        {visibleBackupFailure && (
          <div className="shrink-0 px-10 pb-4 pt-16">
            <BackupFailureAlert
              failure={visibleBackupFailure}
              onDismiss={onDismissBackupFailure}
            />
          </div>
        )}

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
        <AppToaster />
      </div>
  );
}
