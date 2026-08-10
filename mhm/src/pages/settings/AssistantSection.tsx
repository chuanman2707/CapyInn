import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { invokeCommand, invokeWriteCommand } from "@/lib/invokeCommand";
import { useAssistantStore } from "@/stores/useAssistantStore";
import { DELETE_ALL_PHRASE, isDeleteAllPhrase } from "@/types/assistant";
import type { AssistantSettings } from "@/types/assistant";

// Base URL/model ở đây lặp lại đúng giá trị đã khai trong
// agent/assistant/config.rs (DEEPSEEK_BASE_URL, DEEPSEEK_MODEL,
// OPENROUTER_BASE_URL — Task 2). Backend chưa có command trả preset mặc định
// cho frontend nên phải chép tay; hai bản này sẽ trôi nếu Rust đổi mà quên
// sửa ở đây. Chấp nhận được cho phạm vi Task 10, nhưng nên có command riêng
// phục vụ preset về sau thay vì chép tay ở hai nơi.
const PRESETS = [
  {
    value: "deep_seek" as const,
    label: "DeepSeek",
    baseUrl: "https://api.deepseek.com/v1/chat/completions",
    model: "deepseek-chat",
  },
  {
    value: "open_router" as const,
    label: "OpenRouter",
    baseUrl: "https://openrouter.ai/api/v1/chat/completions",
    model: "",
  },
  { value: "custom" as const, label: "Tuỳ chỉnh", baseUrl: "", model: "" },
];

function describeError(caught: unknown): string {
  return caught instanceof Error ? caught.message : String(caught);
}

/// Id riêng, KHÔNG dùng lại id của hộp xoá sạch trong `AssistantHistoryList`:
/// hai màn hình này render cùng lúc được (panel trợ lý là cột giữa của shell,
/// còn đây là tab Cài đặt), và hai phần tử cùng id là một id trùng.
///
/// **TÊN TRUY CẬP cũng phải riêng, vì cùng một lý do.** Bản đầu chỉ tách `id`
/// mà để nguyên nhãn, nên khi cả hai bề mặt cùng render — chuyện xảy ra được
/// ngoài đời, panel là cột giữa còn Cài đặt nằm trong `<main>` — thì trên một
/// màn hình có **hai** nút `Xoá tất cả hội thoại`, **hai** nút `Xoá vĩnh viễn`,
/// **hai** ô `Gõ XOÁ HẾT để xác nhận`. Với người dùng đó là hai nút đỏ y hệt
/// nhau; với test đó là `getByRole` ném lỗi "found multiple elements".
///
/// Cách tách là đổi CÁCH GỌI chứ không chắp `(Cài đặt)` vào đuôi: ở đây thao
/// tác trên **cả sổ** hội thoại (`sổ` là từ đã dùng sẵn trong repo — "sổ hội
/// thoại", "sổ rỗng"), còn hộp trong panel nằm ngay dưới danh sách nên nó nói
/// theo danh sách. Chuỗi `XOÁ HẾT` thì vẫn dùng chung — nó là hàng rào, không
/// phải nhãn (xem `isDeleteAllPhrase` trong `types/assistant.ts`).
const PHRASE_INPUT_ID = "assistant-settings-delete-all-phrase";

export function AssistantSection() {
  const [settings, setSettings] = useState<AssistantSettings | null>(null);
  const [baseUrl, setBaseUrl] = useState("");
  const [model, setModel] = useState("");
  const [preset, setPreset] = useState<AssistantSettings["config"]["preset"]>("deep_seek");
  const [apiKey, setApiKey] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirmingAll, setConfirmingAll] = useState(false);
  const [phrase, setPhrase] = useState("");

  // `busy` của TRỢ LÝ, không phải `busy` của màn hình này. Màn hình Cài đặt
  // không có sẵn nó nên phải chủ động đọc.
  //
  // Trước bản vá, `deleteAllConversations()` gọi `startNewChat()` VÔ ĐIỀU KIỆN
  // → mint khoá phiên mới → kết quả `check_in` đang bay về bị store bỏ (đúng
  // thiết kế của lớp 4), NHƯNG phòng thì đã nhận thật và không màn hình nào nói
  // một chữ. Không mất tiền, mất tin. Đây là cửa xoá sạch thứ năm.
  //
  // Hàng rào thật giờ ở tầng store (bất biến sở hữu `busy` trong
  // `useAssistantStore.ts`), và nó là thứ duy nhất bịt được ca đo được: bấm
  // *Xoá sổ vĩnh viễn* lúc rảnh → lễ tân bấm *Đồng ý* ở panel bên trái →
  // `check_in` bay → lệnh xoá mới về. Cú bấm đã đi rồi thì `disabled` dưới đây
  // không thu lại được. Giữ `disabled` lại vì nó vẫn đúng vai: đừng mời admin
  // bấm một thứ chắc chắn bị từ chối.
  const assistantBusy = useAssistantStore((state) => state.busy);
  const hasPendingAction = useAssistantStore((state) => state.pendingAction !== null);

  // apply() nạp cả bốn field — settings lẫn preset/baseUrl/model — từ phản hồi
  // server. Chỉ hai chỗ được phép gọi nó: lần tải đầu tiên và khi "Lưu cấu
  // hình" thành công. Các thao tác khác (lưu khoá, xoá khoá, bật/tắt công tắc
  // cloud) chỉ đổi phần settings tương ứng của chúng ở backend — nếu cũng gọi
  // apply() thì sẽ ghi đè baseUrl/model đang gõ dở bằng giá trị đã lưu trước
  // đó, xoá mất sửa đổi chưa lưu của chủ khách sạn mà không hề cảnh báo.
  const apply = (next: AssistantSettings) => {
    setSettings(next);
    setPreset(next.config.preset);
    setBaseUrl(next.config.base_url);
    setModel(next.config.model);
  };

  useEffect(() => {
    // Lỗi đọc cấu hình phải lên màn hình: nuốt lỗi ở đây khiến chủ khách sạn
    // thấy form trống và tưởng nhầm là "chưa cấu hình gì", trong khi thật ra
    // app gọi get_assistant_settings thất bại.
    invokeCommand<AssistantSettings>("get_assistant_settings")
      .then(apply)
      .catch((caught) => setError(describeError(caught)));
  }, []);

  const run = async (
    task: () => Promise<AssistantSettings>,
    options: { syncFields?: boolean } = {},
  ) => {
    setBusy(true);
    setError(null);
    try {
      const next = await task();
      if (options.syncFields) {
        apply(next);
      } else {
        // Chỉ cập nhật settings (has_api_key, cloud_data_opt_in, gate) — xem
        // giải thích ở apply() phía trên.
        setSettings(next);
      }
      // Panel trợ lý ở cột thứ ba của shell đọc cổng từ useAssistantStore, còn
      // màn hình này giữ bản sao cục bộ riêng. Store chỉ được nạp đúng một lần
      // lúc MainShell mount, nên không bơm lại ở đây thì chủ khách sạn lưu khoá
      // xong vẫn thấy panel ẩn cho tới lần khởi động lại app — và ngược lại,
      // gỡ cấu hình rồi mà panel vẫn còn. Cùng lối với useRoomConfig.ts:47.
      // refreshSettings tự nuốt lỗi nên không che được lỗi ghi ở catch dưới.
      await useAssistantStore.getState().refreshSettings();
    } catch (caught) {
      setError(describeError(caught));
    } finally {
      setBusy(false);
    }
  };

  const closeDeleteAll = () => {
    setConfirmingAll(false);
    // Dọn luôn chữ đã gõ: để nguyên thì lần mở sau hộp đã sẵn ở trạng thái bật
    // nút, và cú bấm thứ hai không còn phải đi qua hàng rào nào.
    setPhrase("");
  };

  /// Đi qua đúng đường của store chứ không tự gọi `invokeWriteCommand`: sau khi
  /// xoá, phiên đang mở trên panel phải được dọn (`startNewChat`) và danh sách
  /// lịch sử phải nạp lại — chép tay hai bước ấy ở đây là mở một đường thứ hai
  /// để trôi khỏi đường đã được duyệt.
  ///
  /// `try/finally` theo đúng khuôn `run()` ngay trên, và đây không phải kiểu
  /// cách: đo được — cho `deleteAllConversations()` reject thì `setBusy(false)`
  /// và `closeDeleteAll()` KHÔNG BAO GIỜ chạy, `busy` kẹt `true` **vĩnh viễn**,
  /// và cả mục *Trợ lý quầy* chết theo — *Lưu cấu hình*, *Lưu khoá*, *Xoá
  /// khoá*, ô tick đồng ý, cả hai nút xoá đều xám tới khi khởi động lại app.
  /// Đúng hình dạng con bug `busy` kẹt vừa sửa ở tầng store, dựng lại ở tầng
  /// component.
  const deleteAll = async () => {
    setBusy(true);
    setError(null);
    try {
      await useAssistantStore.getState().deleteAllConversations();
      // Store nuốt lỗi vào `error` của CHÍNH NÓ, mà `error` đó chỉ được vẽ
      // trong `AssistantPanel` — panel gần như luôn đóng khi chủ nhà đang ở tab
      // Cài đặt. Không bê sang đây thì `AUTH_FORBIDDEN`, DB khoá, hay câu từ
      // chối vì đang bận đều là im lặng tuyệt đối: bấm xong, không thấy gì,
      // tưởng đã xoá.
      //
      // Đọc nguyên `error` sau lượt chạy chứ không chỉ bắt exception: đường
      // hỏng thứ hai — xoá được nhưng `loadConversations()` sau đó hỏng — cũng
      // phải nói ra, vì lúc ấy danh sách trên panel không còn khớp với đĩa.
      setError(useAssistantStore.getState().error);
    } catch (caught) {
      setError(describeError(caught));
    } finally {
      setBusy(false);
      closeDeleteAll();
    }
  };

  return (
    <section className="space-y-4">
      <div>
        <h3 className="text-lg font-semibold">Trợ lý quầy</h3>
        {/* Kể ĐỦ những đường trợ lý ghi được vào PMS, vì đây là chỗ chủ nhà
            quyết định có bật nó hay không. Bản trước chỉ khai "thẻ xác nhận
            nhận phòng" trong khi trợ lý còn dựng được thẻ đặt phòng trước và
            thẻ ghi bù — người đọc đồng ý cho một quyền hẹp hơn quyền thật.

            Thêm loại thẻ thứ tư thì câu này phải sửa theo; `ACTION_KIND_COPY`
            (`types/assistant.ts`) là danh sách đầy đủ. Cố ý KHÔNG nối bảng ấy
            thành câu: đây là văn xuôi cho người đang cân nhắc bật/tắt đọc, còn
            bảng kia là chữ trên thẻ cho lễ tân đang thao tác. */}
        <p className="text-sm text-brand-muted">
          Trợ lý AI hỗ trợ tra cứu và dựng thẻ xác nhận cho ba việc ghi vào PMS: nhận phòng, đặt
          phòng trước và ghi bù kỳ ở đã qua. Thẻ nào cũng phải có người bấm duyệt. PMS vẫn chạy bình
          thường khi chưa cấu hình.
        </p>
      </div>

      <label className="block text-sm">
        Nhà cung cấp
        <select
          aria-label="Nhà cung cấp"
          className="mt-1 w-full rounded-xl border border-slate-200 px-3 py-2"
          value={preset}
          onChange={(event) => {
            const chosen = PRESETS.find((item) => item.value === event.target.value);
            if (!chosen) return;
            setPreset(chosen.value);
            setBaseUrl(chosen.baseUrl);
            setModel(chosen.model);
          }}
        >
          {PRESETS.map((item) => (
            <option key={item.value} value={item.value}>
              {item.label}
            </option>
          ))}
        </select>
      </label>

      <label className="block text-sm">
        Địa chỉ máy chủ
        <input
          className="mt-1 w-full rounded-xl border border-slate-200 px-3 py-2"
          value={baseUrl}
          onChange={(event) => setBaseUrl(event.target.value)}
        />
      </label>

      <label className="block text-sm">
        Model
        <input
          className="mt-1 w-full rounded-xl border border-slate-200 px-3 py-2"
          value={model}
          onChange={(event) => setModel(event.target.value)}
        />
        <span className="text-xs text-brand-muted">
          Model phải hỗ trợ gọi công cụ. `deepseek-reasoner` thì không.
        </span>
      </label>

      <Button
        disabled={busy}
        onClick={() =>
          run(
            () =>
              invokeWriteCommand<AssistantSettings>("set_assistant_settings", {
                preset,
                baseUrl,
                model,
              }),
            { syncFields: true },
          )
        }
      >
        Lưu cấu hình
      </Button>

      <div className="rounded-xl bg-slate-50 p-3">
        <p className="text-sm">
          Khoá API: <strong>{settings?.has_api_key ? "đã cấu hình" : "chưa cấu hình"}</strong>
        </p>
        <label className="mt-2 block text-sm" htmlFor="assistant-api-key">
          Khoá API
        </label>
        <input
          id="assistant-api-key"
          type="password"
          className="mt-1 w-full rounded-xl border border-slate-200 px-3 py-2"
          value={apiKey}
          onChange={(event) => setApiKey(event.target.value)}
        />
        <div className="mt-2 flex gap-2">
          <Button
            disabled={busy || !apiKey.trim()}
            onClick={() =>
              run(async () => {
                const next = await invokeWriteCommand<AssistantSettings>("set_assistant_api_key", {
                  apiKey,
                });
                setApiKey("");
                return next;
              })
            }
          >
            Lưu khoá
          </Button>
          <Button
            variant="ghost"
            disabled={busy || !settings?.has_api_key}
            onClick={() =>
              run(() => invokeWriteCommand<AssistantSettings>("clear_assistant_api_key"))
            }
          >
            Xoá khoá
          </Button>
        </div>
      </div>

      <div className="rounded-xl border border-amber-200 bg-amber-50 p-3">
        <label className="flex items-start gap-2 text-sm">
          <input
            type="checkbox"
            className="mt-1"
            checked={settings?.cloud_data_opt_in ?? false}
            disabled={busy}
            onChange={(event) =>
              run(() =>
                invokeWriteCommand<AssistantSettings>("set_assistant_cloud_opt_in", {
                  enabled: event.target.checked,
                }),
              )
            }
          />
          {/* Câu đồng ý phải nói CẢ HAI vế. Bản cũ chỉ nói vế đi: dữ liệu rời
              khỏi máy này. Nhưng chủ nhà đã chọn "giữ nguyên văn, không tự
              xoá", nên mỗi lượt chat còn để lại một bản sao Ở LẠI máy quầy —
              tên khách và số CCCD, tích luỹ vô thời hạn. Đồng ý mà chỉ được kể
              vế đi là đồng ý cho một thứ khác với thứ thật sự xảy ra.

              Cả hai vế nằm trong chính NHÃN của ô tick, không phải ở một đoạn
              chú thích cạnh bên: thứ chủ nhà đang đồng ý là cái nhãn này. */}
          <span>
            Đồng ý gửi dữ liệu lên máy chủ AI. Khi bật, dữ liệu khách — gồm tên và số giấy tờ — sẽ
            <strong> rời khỏi máy này</strong> để gửi tới nhà cung cấp đã chọn, và cũng
            <strong> ở lại máy này</strong> dưới dạng hội thoại đã lưu. Phần mềm
            <strong> không tự xoá</strong> hội thoại — chúng nằm lại cho tới khi admin xoá tay. Tắt
            công tắc chỉ dừng việc gửi đi, không xoá phần đã lưu.
          </span>
        </label>
      </div>

      {/* Cửa xoá sạch THỨ HAI mà spec dòng 359 đòi: nút *Xoá tất cả* phải có ở
          **cả** cuối danh sách lịch sử **và** ở đây. Lý do nó là bắt buộc chứ
          không phải trang trí nằm ngay trên bảng của spec: "Vì không tự xoá,
          đường xoá tay là lối ra DUY NHẤT và phải với tới được." Bắt admin mở
          panel trợ lý, vào lịch sử, cuộn xuống cuối mới xoá được là chôn lối ra
          đó sau ba cú bấm trong một màn hình không ai vào để dọn dẹp.

          Section này chỉ render khi admin (`settings/index.tsx:144`) nên nút
          không tự kiểm quyền để hiện. Đó là lớp LỊCH SỰ, không phải hàng rào:
          `delete_all_assistant_conversations` là admin-only phía Rust và trả
          `AUTH_FORBIDDEN` cho lễ tân.

          Cố ý KHÔNG ẩn nút khi sổ rỗng (danh sách lịch sử thì có ẩn): màn hình
          này không tải danh sách hội thoại, nên "sổ rỗng" ở đây chỉ đoán được
          bằng cách gọi thêm một lệnh đọc mỗi lần mở tab Cài đặt — trả một cú
          gọi thường trực để lấy về một cú bấm vô hại. */}
      <div className="rounded-xl border border-red-200 bg-red-50 p-3">
        <p className="text-sm font-semibold">Xoá toàn bộ hội thoại trợ lý</p>
        <p className="mt-1 text-xs text-red-700">
          Hội thoại được giữ nguyên văn — gồm tên khách và số giấy tờ — và hệ thống không tự xoá.
          Đây là đường xoá tay duy nhất.
        </p>

        {confirmingAll ? (
          <div className="mt-3 space-y-3">
            <p className="text-sm font-semibold">Xoá sạch sổ hội thoại trợ lý?</p>
            <p className="text-xs text-red-700">
              Không hoàn tác. Ngoài bản sao lưu ở Data &amp; Backup thì đây là bản duy nhất của cả
              sổ.
            </p>

            {/* Chỉ một câu, và có điều kiện — câu cảnh báo thường trực là câu
                người ta thôi đọc. Xoá sạch dọn luôn phiên đang mở trên panel
                nên thẻ đang chờ mất theo, kể cả khi lệnh xoá không xoá nổi một
                dòng nào của chính hội thoại đang mở.

                "trên panel" không phải chữ thừa: người đọc câu này đang đứng ở
                tab Cài đặt, còn cái thẻ thì nằm ở cột bên kia màn hình.

                TRUNG TÍNH với loại thẻ: `hasPendingAction` là một `boolean`,
                chỗ này không biết — và không cần biết — thẻ đang treo là nhận
                phòng, đặt phòng hay ghi bù. Gọi bừa nó là "thẻ nhận phòng" như
                bản trước thì hai phần ba số lần là nói sai về thứ sắp mất. Muốn
                nói đúng tên thì phải chuyền `kind` qua hai lớp prop, đổi lấy
                một danh từ mà người đọc đang không cần. */}
            {hasPendingAction && (
              <p className="text-xs font-medium text-red-700">
                Thẻ đang chờ duyệt trên panel cũng sẽ mất.
              </p>
            )}

            <label htmlFor={PHRASE_INPUT_ID} className="block text-xs text-red-700">
              Gõ {DELETE_ALL_PHRASE} để xoá sổ
            </label>
            <input
              id={PHRASE_INPUT_ID}
              type="text"
              value={phrase}
              onChange={(event) => setPhrase(event.target.value)}
              className="w-full rounded-xl border border-red-200 bg-white px-3 py-2 text-sm outline-none focus:border-red-400"
            />
            <div className="flex gap-2">
              <Button
                size="sm"
                className="bg-red-600 text-white"
                disabled={busy || assistantBusy || !isDeleteAllPhrase(phrase)}
                onClick={() => void deleteAll()}
              >
                Xoá sổ vĩnh viễn
              </Button>
              <Button size="sm" variant="outline" onClick={closeDeleteAll}>
                Giữ lại sổ
              </Button>
            </div>
          </div>
        ) : (
          <Button
            size="sm"
            variant="ghost"
            className="mt-2 text-red-600"
            disabled={busy || assistantBusy}
            onClick={() => setConfirmingAll(true)}
          >
            Xoá sổ hội thoại trợ lý
          </Button>
        )}
      </div>

      {error && <p className="text-sm text-red-600">{error}</p>}
    </section>
  );
}
