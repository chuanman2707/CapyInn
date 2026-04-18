import { Badge } from "@/components/ui/badge";
import type { AppUpdatePhase } from "@/types";

interface AppUpdateBadgeProps {
  phase: AppUpdatePhase;
  onCheckOrDownload: () => void | Promise<void>;
  onRestart: () => void;
}

export default function AppUpdateBadge({
  phase,
  onCheckOrDownload,
  onRestart,
}: AppUpdateBadgeProps) {
  if (phase !== "available" && phase !== "downloading" && phase !== "downloaded") {
    return null;
  }

  const label =
    phase === "available"
      ? "UPDATE"
      : phase === "downloading"
        ? "DOWNLOADING..."
        : "RESTART TO UPDATE";

  const handleClick = () => {
    if (phase === "downloaded") {
      onRestart();
      return;
    }

    void onCheckOrDownload();
  };

  return (
    <button
      type="button"
      onClick={handleClick}
      disabled={phase === "downloading"}
      className="cursor-pointer disabled:cursor-not-allowed"
    >
      <Badge className="bg-amber-50 text-amber-700 border-0 rounded-full py-1.5 px-3 uppercase tracking-wider text-[10px] font-bold">
        {label}
      </Badge>
    </button>
  );
}
