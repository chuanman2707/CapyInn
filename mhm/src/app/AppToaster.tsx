import { Toaster } from "sonner";

export function AppToaster() {
  return (
    <Toaster
      position="bottom-right"
      toastOptions={{ className: "rounded-xl shadow-float font-sans" }}
    />
  );
}
