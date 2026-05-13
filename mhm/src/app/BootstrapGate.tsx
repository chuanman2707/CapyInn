import type { ReactNode } from "react";

import { AppToaster } from "@/app/AppToaster";
import { useBootstrapState } from "@/app/BootstrapState";
import OnboardingWizard from "@/pages/onboarding";
import type { BootstrapStatus } from "@/types";

export function BootstrapGate({
  children,
}: {
  children: (state: { bootstrap: BootstrapStatus | null }) => ReactNode;
}) {
  const { bootstrap, bootstrapLoading, completeOnboarding } = useBootstrapState();

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
        <OnboardingWizard onCompleted={completeOnboarding} />
        <AppToaster />
      </>
    );
  }

  return <>{children({ bootstrap })}</>;
}
