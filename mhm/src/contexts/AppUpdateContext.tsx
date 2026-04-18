import { createContext, useContext, type ReactNode } from "react";

import { useAppUpdateController } from "@/hooks/useAppUpdateController";

type AppUpdateContextValue = ReturnType<typeof useAppUpdateController>;

const AppUpdateContext = createContext<AppUpdateContextValue | null>(null);

export function AppUpdateProvider({
  value,
  children,
}: {
  value: AppUpdateContextValue;
  children: ReactNode;
}) {
  return <AppUpdateContext.Provider value={value}>{children}</AppUpdateContext.Provider>;
}

export function useAppUpdate() {
  const value = useContext(AppUpdateContext);

  if (!value) {
    throw new Error("useAppUpdate must be used inside AppUpdateProvider");
  }

  return value;
}
