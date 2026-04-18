import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { BackupStatusIndicator } from "./BackupStatusIndicator";

describe("BackupStatusIndicator", () => {
  it("returns nothing when hidden", () => {
    const { container } = render(
      <BackupStatusIndicator visible={false} phase="saving" message="Backing up" />,
    );

    expect(container.firstChild).toBeNull();
  });

  it("shows the message when visible", () => {
    render(<BackupStatusIndicator visible phase="saving" message="Backing up settings" />);

    expect(screen.getByText("Backing up settings")).toBeInTheDocument();
  });

  it("renders the saving state with a spinner icon", () => {
    const { container } = render(
      <BackupStatusIndicator visible phase="saving" message="Backing up settings" />,
    );

    expect(container.firstChild).toHaveAttribute("data-phase", "saving");
    expect(screen.getByTestId("backup-status-icon")).toHaveClass("animate-spin");
  });

  it("renders the saved state with a success icon", () => {
    const { container } = render(
      <BackupStatusIndicator visible phase="saved" message="Backup saved" />,
    );

    expect(container.firstChild).toHaveAttribute("data-phase", "saved");
    expect(screen.getByTestId("backup-status-icon")).toHaveClass("text-emerald-600");
  });

  it("renders the failed state with an error icon", () => {
    const { container } = render(
      <BackupStatusIndicator visible phase="failed" message="Backup failed" />,
    );

    expect(container.firstChild).toHaveAttribute("data-phase", "failed");
    expect(screen.getByTestId("backup-status-icon")).toHaveClass("text-rose-600");
  });

  it("anchors the indicator at the top right", () => {
    const { container } = render(
      <BackupStatusIndicator visible phase="saved" message="Backup saved" />,
    );

    expect(container.firstChild).toHaveClass("fixed", "right-4");
  });
});
