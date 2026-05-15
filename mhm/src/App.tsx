import { AppUpdateRuntime } from "@/app/AppUpdateRuntime";
import { AuthGate } from "@/app/AuthGate";
import { BootstrapGate } from "@/app/BootstrapGate";
import { BootstrapStateProvider } from "@/app/BootstrapState";
import { MainShell } from "@/app/MainShell";
import { RuntimeStateProvider } from "@/app/RuntimeStateProvider";

export default function App() {
  return (
    <BootstrapStateProvider>
      <AppUpdateRuntime>
        <RuntimeStateProvider>
          <BootstrapGate>
            {({ bootstrap }) => (
              <AuthGate bootstrap={bootstrap}>
                <MainShell />
              </AuthGate>
            )}
          </BootstrapGate>
        </RuntimeStateProvider>
      </AppUpdateRuntime>
    </BootstrapStateProvider>
  );
}
