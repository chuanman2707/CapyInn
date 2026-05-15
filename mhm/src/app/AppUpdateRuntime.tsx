import { useEffect, useRef, type ReactNode } from "react";

import { useBootstrapState } from "@/app/BootstrapState";
import { AppUpdateProvider } from "@/contexts/AppUpdateContext";
import { useAppUpdateController } from "@/hooks/useAppUpdateController";

export function AppUpdateRuntime({ children }: { children: ReactNode }) {
  const { shellReady } = useBootstrapState();
  const didAutoCheckRef = useRef(false);
  const appUpdate = useAppUpdateController({
    enabled: shellReady,
    supported: __UPDATER_ENABLED__,
    currentVersion: __APP_VERSION__,
  });

  useEffect(() => {
    if (!shellReady || didAutoCheckRef.current) {
      return;
    }

    didAutoCheckRef.current = true;
    void appUpdate.checkForUpdates({ silent: true });
  }, [appUpdate, shellReady]);

  return <AppUpdateProvider value={appUpdate}>{children}</AppUpdateProvider>;
}
