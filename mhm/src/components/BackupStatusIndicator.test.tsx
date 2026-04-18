import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { BackupStatusIndicator } from "./BackupStatusIndicator";
import type { BackupStatusPayload } from "@/types";

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

  it("renders the saving state with a cloud cog icon", () => {
    const { container } = render(
      <BackupStatusIndicator visible phase="saving" message="Backing up settings" />,
    );

    expect(container.firstChild).toHaveAttribute("data-phase", "saving");
    expect(screen.getByTestId("backup-status-icon")).toHaveAttribute("data-icon", "cloud-cog");
    expect(screen.getByTestId("backup-status-icon")).toHaveClass("bg-brand-primary/10");
  });

  it("renders the saved state with a cloud check icon", () => {
    const { container } = render(
      <BackupStatusIndicator visible phase="saved" message="Backup saved" />,
    );

    expect(container.firstChild).toHaveAttribute("data-phase", "saved");
    expect(screen.getByTestId("backup-status-icon")).toHaveAttribute("data-icon", "cloud-check");
    expect(screen.getByTestId("backup-status-icon")).toHaveClass("text-emerald-600");
  });

  it("renders the failed state with a cloud alert icon", () => {
    const { container } = render(
      <BackupStatusIndicator visible phase="failed" message="Backup failed" />,
    );

    expect(container.firstChild).toHaveAttribute("data-phase", "failed");
    expect(screen.getByTestId("backup-status-icon")).toHaveAttribute("data-icon", "cloud-alert");
    expect(screen.getByTestId("backup-status-icon")).toHaveClass("text-rose-600");
  });

  it("anchors the indicator at the top right", () => {
    const { container } = render(
      <BackupStatusIndicator visible phase="saved" message="Backup saved" />,
    );

    expect(container.firstChild).toHaveClass("fixed", "right-4");
  });

  it("accepts backend backup event states", () => {
    const payload = {
      job_id: "job-1",
      state: "started",
      reason: "manual",
      pending_jobs: 0,
    } satisfies BackupStatusPayload;

    expect(payload.state).toBe("started");
  });
});
