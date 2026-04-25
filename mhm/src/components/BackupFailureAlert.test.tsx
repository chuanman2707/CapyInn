import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { BackupFailureAlert } from "./BackupFailureAlert";

describe("BackupFailureAlert", () => {
  it("renders a distinct alert with title, backend message, and source label", () => {
    render(
      <BackupFailureAlert
        failure={{ jobId: "job-1", reason: "manual", message: "Ổ đĩa đầy" }}
        onDismiss={() => undefined}
      />,
    );

    const alert = screen.getByRole("alert", { name: "Sao lưu thất bại" });
    expect(within(alert).getByText("Sao lưu thất bại")).toBeInTheDocument();
    expect(within(alert).getByText("Ổ đĩa đầy")).toBeInTheDocument();
    expect(within(alert).getByText("Nguồn: Thủ công")).toBeInTheDocument();
  });

  it("uses the fallback message when the backend message is blank", () => {
    render(
      <BackupFailureAlert
        failure={{ jobId: "job-1", reason: "scheduled", message: "   " }}
        onDismiss={() => undefined}
      />,
    );

    const alert = screen.getByRole("alert", { name: "Sao lưu thất bại" });
    expect(within(alert).getByText("Sao lưu thất bại")).toBeInTheDocument();
    expect(
      within(alert).getByText(
        "Không thể tạo bản sao lưu. Vui lòng kiểm tra dung lượng ổ đĩa hoặc thử lại.",
      ),
    ).toBeInTheDocument();
    expect(within(alert).getByText("Nguồn: Tự động")).toBeInTheDocument();
  });

  it("calls onDismiss with the failed job id", () => {
    const onDismiss = vi.fn();

    render(
      <BackupFailureAlert
        failure={{ jobId: "job-42", reason: "checkout", message: "Không thể ghi file" }}
        onDismiss={onDismiss}
      />,
    );

    const alert = screen.getByRole("alert", { name: "Sao lưu thất bại" });
    expect(within(alert).getByText("Sao lưu thất bại")).toBeInTheDocument();
    expect(within(alert).getByText("Nguồn: Trả phòng")).toBeInTheDocument();

    fireEvent.click(within(alert).getByRole("button", { name: "Đóng cảnh báo sao lưu" }));

    expect(onDismiss).toHaveBeenCalledTimes(1);
    expect(onDismiss).toHaveBeenCalledWith("job-42");
  });
});
