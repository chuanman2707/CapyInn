import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import AppUpdateBadge from "./AppUpdateBadge";

describe("AppUpdateBadge", () => {
  it("renders nothing when no update action is available", () => {
    const { container } = render(
      <AppUpdateBadge phase="idle" onCheckOrDownload={vi.fn()} onRestart={vi.fn()} />,
    );

    expect(container.firstChild).toBeNull();
  });

  it("shows UPDATE when an update is available", () => {
    render(
      <AppUpdateBadge phase="available" onCheckOrDownload={vi.fn()} onRestart={vi.fn()} />,
    );

    expect(screen.getByRole("button", { name: "UPDATE" })).toBeInTheDocument();
  });

  it("shows DOWNLOADING... while the update is downloading", () => {
    render(
      <AppUpdateBadge phase="downloading" onCheckOrDownload={vi.fn()} onRestart={vi.fn()} />,
    );

    expect(screen.getByRole("button", { name: "DOWNLOADING..." })).toBeDisabled();
  });

  it("shows RESTART TO UPDATE after download completes", () => {
    render(
      <AppUpdateBadge phase="downloaded" onCheckOrDownload={vi.fn()} onRestart={vi.fn()} />,
    );

    expect(screen.getByRole("button", { name: "RESTART TO UPDATE" })).toBeInTheDocument();
  });

  it("downloads when the update badge is clicked", async () => {
    const user = userEvent.setup();
    const onCheckOrDownload = vi.fn();

    render(
      <AppUpdateBadge
        phase="available"
        onCheckOrDownload={onCheckOrDownload}
        onRestart={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: "UPDATE" }));

    expect(onCheckOrDownload).toHaveBeenCalledTimes(1);
  });

  it("reopens the restart prompt when the restart badge is clicked", async () => {
    const user = userEvent.setup();
    const onRestart = vi.fn();

    render(
      <AppUpdateBadge phase="downloaded" onCheckOrDownload={vi.fn()} onRestart={onRestart} />,
    );

    await user.click(screen.getByRole("button", { name: "RESTART TO UPDATE" }));

    expect(onRestart).toHaveBeenCalledTimes(1);
  });
});
