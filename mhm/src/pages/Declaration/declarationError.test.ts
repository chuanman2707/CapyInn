import { describe, expect, it } from "vitest";

import { declarationErrorMessage } from "./declarationError";

describe("declarationErrorMessage", () => {
  // FINDING I1/I2: các lệnh kbtt_* trả Err(String) thẳng, không qua registry
  // AppError dùng chung — formatAppError() sẽ nuốt mất chuỗi này và trả về
  // câu chung chung "Có lỗi hệ thống, vui lòng thử lại", đúng lúc người vận
  // hành cần đọc được lý do thật nhất (số giấy tờ trùng ai, khách đang ở đâu).
  it("giữ nguyên chuỗi lỗi tiếng Việt trần từ lệnh Tauri", () => {
    const raw =
      "Số giấy tờ 058195006173 đang thuộc về một khách khác trong hệ thống: " +
      "Phan Thị Mỹ Hà (sinh 1995-07-28). Kiểm tra lại số giấy tờ vừa nhập.";

    expect(declarationErrorMessage(raw)).toBe(raw);
  });

  it("vẫn dùng formatAppError cho lỗi có hình dạng registry chuẩn", () => {
    const structured = {
      code: "AUTH_INVALID_PIN",
      message: "Mã PIN không đúng",
      kind: "user" as const,
      support_id: null,
    };

    expect(declarationErrorMessage(structured)).toBe("Mã PIN không đúng");
  });
});
