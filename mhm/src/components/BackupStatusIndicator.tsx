import { CloudAlert, CloudCheck, CloudCog } from "lucide-react";

import type { BackupIndicatorPhase } from "@/types";
import { cn } from "@/lib/utils";

type BackupStatusIndicatorProps = {
  visible: boolean;
  phase: BackupIndicatorPhase;
  message: string;
};

const PHASE_CONFIG: Record<
  BackupIndicatorPhase,
  {
    iconName: string;
    icon: typeof CloudCog;
    iconClassName: string;
    toneClassName: string;
  }
> = {
  saving: {
    iconName: "cloud-cog",
    icon: CloudCog,
    iconClassName: "animate-spin",
    toneClassName: "bg-brand-primary/10 text-brand-primary",
  },
  saved: {
    iconName: "cloud-check",
    icon: CloudCheck,
    iconClassName: "",
    toneClassName: "bg-emerald-50 text-emerald-600",
  },
  failed: {
    iconName: "cloud-alert",
    icon: CloudAlert,
    iconClassName: "",
    toneClassName: "bg-rose-50 text-rose-600",
  },
};

export function BackupStatusIndicator({ visible, phase, message }: BackupStatusIndicatorProps) {
  if (!visible) return null;

  const config = PHASE_CONFIG[phase];
  const Icon = config.icon;

  return (
    <div
      aria-live="polite"
      data-phase={phase}
      role="status"
      className="fixed right-4 top-24 z-40 pointer-events-none max-w-[calc(100vw-2rem)]"
    >
      <div
        className={cn(
          "pointer-events-none inline-flex items-center gap-2 rounded-full border border-slate-200/80 bg-white/95 px-4 py-2.5 text-sm font-medium text-brand-text shadow-float backdrop-blur-md",
          "animate-fade-up",
        )}
      >
        <span
          data-testid="backup-status-icon"
          data-icon={config.iconName}
          className={cn(
            "inline-flex h-8 w-8 items-center justify-center rounded-full",
            config.toneClassName,
          )}
        >
          <Icon aria-hidden="true" size={16} className={cn("shrink-0", config.iconClassName)} />
        </span>
        <span className="max-w-[22rem] truncate">{message}</span>
      </div>
    </div>
  );
}
