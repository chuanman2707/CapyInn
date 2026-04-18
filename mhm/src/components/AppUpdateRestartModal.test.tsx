import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import AppUpdateRestartModal from "./AppUpdateRestartModal";

describe("AppUpdateRestartModal", () => {
  it("renders nothing when closed", () => {
    const { container } = render(
      <AppUpdateRestartModal
        open={false}
        version="0.2.0"
        onConfirm={vi.fn()}
        onLater={vi.fn()}
      />,
    );

    expect(container.firstChild).toBeNull();
  });

  it("shows the restart prompt after download completes", () => {
    render(
      <AppUpdateRestartModal
        open
        version="0.2.0"
        onConfirm={vi.fn()}
        onLater={vi.fn()}
      />,
    );

    expect(screen.getByText("Restart to update")).toBeInTheDocument();
    expect(screen.getByText(/Version 0.2.0 is ready to install/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Later" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Restart to update" })).toBeInTheDocument();
  });

  it("fires the later action", async () => {
    const user = userEvent.setup();
    const onLater = vi.fn();

    render(
      <AppUpdateRestartModal
        open
        version="0.2.0"
        onConfirm={vi.fn()}
        onLater={onLater}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Later" }));

    expect(onLater).toHaveBeenCalledTimes(1);
  });

  it("fires the restart action", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();

    render(
      <AppUpdateRestartModal
        open
        version="0.2.0"
        onConfirm={onConfirm}
        onLater={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Restart to update" }));

    expect(onConfirm).toHaveBeenCalledTimes(1);
  });
});
