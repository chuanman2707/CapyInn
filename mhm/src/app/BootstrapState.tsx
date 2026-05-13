import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";

import { useAuthStore } from "@/stores/useAuthStore";
import type { BootstrapStatus } from "@/types";

type BootstrapStateValue = {
  bootstrap: BootstrapStatus | null;
  bootstrapLoading: boolean;
  completeOnboarding: (status: BootstrapStatus) => void;
  shellReady: boolean;
};

const BootstrapStateContext = createContext<BootstrapStateValue | null>(null);

export function BootstrapStateProvider({ children }: { children: ReactNode }) {
  const hydrateFromBootstrap = useAuthStore((state) => state.hydrateFromBootstrap);
  const isAuthenticated = useAuthStore((state) => state.isAuthenticated);
  const [bootstrap, setBootstrap] = useState<BootstrapStatus | null>(null);
  const [bootstrapLoading, setBootstrapLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;

    invoke<BootstrapStatus>("get_bootstrap_status")
      .then((status) => {
        if (cancelled) {
          return;
        }

        setBootstrap(status);
        if (status.current_user) {
          hydrateFromBootstrap(status.current_user);
        }
      })
      .finally(() => {
        if (!cancelled) {
          setBootstrapLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [hydrateFromBootstrap]);

  function completeOnboarding(status: BootstrapStatus) {
    setBootstrap(status);
    hydrateFromBootstrap(status.current_user);
  }

  const shellReady =
    !bootstrapLoading &&
    Boolean(bootstrap?.setup_completed) &&
    (!bootstrap?.app_lock_enabled || isAuthenticated);

  const value = useMemo(
    () => ({
      bootstrap,
      bootstrapLoading,
      completeOnboarding,
      shellReady,
    }),
    [bootstrap, bootstrapLoading, shellReady],
  );

  return (
    <BootstrapStateContext.Provider value={value}>
      {children}
    </BootstrapStateContext.Provider>
  );
}

export function useBootstrapState() {
  const value = useContext(BootstrapStateContext);

  if (!value) {
    throw new Error("useBootstrapState must be used inside BootstrapStateProvider");
  }

  return value;
}
