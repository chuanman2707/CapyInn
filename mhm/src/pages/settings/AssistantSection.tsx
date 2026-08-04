import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { invokeCommand, invokeWriteCommand } from "@/lib/invokeCommand";
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

export function AssistantSection() {
  const [settings, setSettings] = useState<AssistantSettings | null>(null);
  const [baseUrl, setBaseUrl] = useState("");
  const [model, setModel] = useState("");
  const [preset, setPreset] = useState<AssistantSettings["config"]["preset"]>("deep_seek");
  const [apiKey, setApiKey] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

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

  const run = async (task: () => Promise<AssistantSettings>) => {
    setBusy(true);
    setError(null);
    try {
      apply(await task());
    } catch (caught) {
      setError(describeError(caught));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="space-y-4">
      <div>
        <h3 className="text-lg font-semibold">Trợ lý quầy</h3>
        <p className="text-sm text-brand-muted">
          Trợ lý AI hỗ trợ tra cứu và dựng thẻ xác nhận nhận phòng. PMS vẫn chạy bình thường khi
          chưa cấu hình.
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
          run(() =>
            invokeWriteCommand<AssistantSettings>("set_assistant_settings", {
              preset,
              baseUrl,
              model,
            }),
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
            aria-label="Đồng ý gửi dữ liệu khách lên máy chủ AI"
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
          <span>
            Đồng ý gửi dữ liệu lên máy chủ AI. Khi bật, dữ liệu khách — gồm tên và số giấy tờ — sẽ
            <strong> rời khỏi máy này</strong> để gửi tới nhà cung cấp đã chọn. Tắt lại bất cứ lúc
            nào.
          </span>
        </label>
      </div>

      {error && <p className="text-sm text-red-600">{error}</p>}
    </section>
  );
}
