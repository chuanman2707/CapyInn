import { AppUpdateRuntime } from "@/app/AppUpdateRuntime";
import { AuthGate } from "@/app/AuthGate";
import { BootstrapGate } from "@/app/BootstrapGate";
import { BootstrapStateProvider } from "@/app/BootstrapState";
import { MainShell } from "@/app/MainShell";
import { RuntimeStateProvider } from "@/app/RuntimeStateProvider";
import { isExperimentalGatewayUiEnabled } from "@/app/runtimeProfile";

export default function App() {
  const experimentalGatewayUi = isExperimentalGatewayUiEnabled();

  return (
    <BootstrapStateProvider>
      <AppUpdateRuntime>
        <RuntimeStateProvider experimentalGatewayUi={experimentalGatewayUi}>
          <BootstrapGate>
            {({ bootstrap }) => (
              <AuthGate bootstrap={bootstrap}>
                <MainShell experimentalGatewayUi={experimentalGatewayUi} />
              </AuthGate>
            )}
          </BootstrapGate>
        </RuntimeStateProvider>
      </AppUpdateRuntime>
    </BootstrapStateProvider>
  );
}
