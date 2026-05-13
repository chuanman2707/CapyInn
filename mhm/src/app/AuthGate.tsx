import { useEffect, type ReactNode } from "react";

import { AppToaster } from "@/app/AppToaster";
import LoginScreen from "@/pages/LoginScreen";
import { useAuthStore } from "@/stores/useAuthStore";
import type { BootstrapStatus } from "@/types";

export function AuthGate({
  bootstrap,
  children,
}: {
  bootstrap: BootstrapStatus | null;
  children: ReactNode;
}) {
  const isAuthenticated = useAuthStore((state) => state.isAuthenticated);
  const checkSession = useAuthStore((state) => state.checkSession);

  useEffect(() => {
    if (!bootstrap?.app_lock_enabled) {
      return;
    }

    checkSession();
  }, [bootstrap?.app_lock_enabled, checkSession]);

  if (bootstrap?.app_lock_enabled && !isAuthenticated) {
    return (
      <>
        <LoginScreen />
        <AppToaster />
      </>
    );
  }

  return <>{children}</>;
}
