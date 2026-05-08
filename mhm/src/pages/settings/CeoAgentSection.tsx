import { useEffect, useState } from "react";
import { toast } from "sonner";

import { invokeCommand, invokeWriteCommand } from "@/lib/invokeCommand";

type CeoTelegramConfig = {
  runtime_enabled: boolean;
  telegram_user_id: string | null;
  telegram_bot_token_present: boolean;
  openai_api_key_present: boolean;
  openai_model: string;
  last_update_id: number | null;
};

type CeoTelegramGateStatus = {
  ready: boolean;
  missing: string[];
};

type CeoDigestConfig = {
  digest_enabled: boolean;
  telegram_user_id: string | null;
  telegram_delivery_chat_id: number | null;
  telegram_bot_token_present: boolean;
  openai_api_key_present: boolean;
  openai_model: string;
};

type CeoDigestGateStatus = {
  ready: boolean;
  missing: string[];
};

const DEFAULT_CONFIG: CeoTelegramConfig = {
  runtime_enabled: false,
  telegram_user_id: null,
  telegram_bot_token_present: false,
  openai_api_key_present: false,
  openai_model: "gpt-5",
  last_update_id: null,
};

const DEFAULT_DIGEST_CONFIG: CeoDigestConfig = {
  digest_enabled: false,
  telegram_user_id: null,
  telegram_delivery_chat_id: null,
  telegram_bot_token_present: false,
  openai_api_key_present: false,
  openai_model: "gpt-5",
};

const GATE_LABELS: Record<string, string> = {
  cloud_data_opt_in: "CEO cloud-data opt-in",
  runtime_enabled: "Runtime enabled",
  telegram_owner_binding: "Telegram owner binding",
  telegram_bot_token: "Telegram bot token",
  open_ai_api_key: "OpenAI API key",
};

const DIGEST_GATE_LABELS: Record<string, string> = {
  digest_enabled: "Digest enabled",
  telegram_delivery_chat_id: "Telegram delivery chat ID",
  cloud_data_opt_in: "CEO cloud-data opt-in",
  telegram_owner_binding: "Telegram owner binding",
  telegram_bot_token: "Telegram bot token",
  open_ai_api_key: "OpenAI API key",
  open_ai_model: "OpenAI model",
};

export default function CeoAgentSection() {
  const [cloudOptIn, setCloudOptIn] = useState(false);
  const [config, setConfig] = useState<CeoTelegramConfig>(DEFAULT_CONFIG);
  const [gateStatus, setGateStatus] = useState<CeoTelegramGateStatus>({
    ready: false,
    missing: ["runtime_enabled"],
  });
  const [digestConfig, setDigestConfig] = useState<CeoDigestConfig>(DEFAULT_DIGEST_CONFIG);
  const [digestGateStatus, setDigestGateStatus] = useState<CeoDigestGateStatus>({
    ready: false,
    missing: ["digest_enabled"],
  });
  const [telegramUserId, setTelegramUserId] = useState("");
  const [telegramDeliveryChatId, setTelegramDeliveryChatId] = useState("");
  const [openaiModel, setOpenaiModel] = useState(DEFAULT_CONFIG.openai_model);
  const [telegramBotToken, setTelegramBotToken] = useState("");
  const [openaiApiKey, setOpenaiApiKey] = useState("");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [statusMessage, setStatusMessage] = useState<string | null>(null);

  const applyConfig = (nextConfig: CeoTelegramConfig) => {
    setConfig(nextConfig);
    setTelegramUserId(nextConfig.telegram_user_id ?? "");
    setOpenaiModel(nextConfig.openai_model);
  };

  const applyDigestConfig = (nextConfig: CeoDigestConfig) => {
    setDigestConfig(nextConfig);
    setTelegramDeliveryChatId(nextConfig.telegram_delivery_chat_id?.toString() ?? "");
  };

  const applyDigestCredentialState = (nextConfig: CeoDigestConfig) => {
    setDigestConfig((current) => ({
      ...current,
      telegram_user_id: nextConfig.telegram_user_id,
      telegram_bot_token_present: nextConfig.telegram_bot_token_present,
      openai_api_key_present: nextConfig.openai_api_key_present,
      openai_model: nextConfig.openai_model,
    }));
  };

  const refreshDigestGateStatus = async () => {
    const nextGate = await invokeCommand<CeoDigestGateStatus>("get_ceo_digest_gate_status");
    setDigestGateStatus(nextGate);
  };

  const refreshDigestGateStatusBestEffort = async () => {
    try {
      await refreshDigestGateStatus();
    } catch {
      toast.error("Unable to refresh CEO Hourly Digest status");
    }
  };

  const refreshDigestCredentialStateAndGateBestEffort = async () => {
    try {
      const [nextDigestConfig, nextDigestGate] = await Promise.all([
        invokeCommand<CeoDigestConfig>("get_ceo_digest_config"),
        invokeCommand<CeoDigestGateStatus>("get_ceo_digest_gate_status"),
      ]);
      applyDigestCredentialState(nextDigestConfig);
      setDigestGateStatus(nextDigestGate);
    } catch {
      toast.error("Unable to refresh CEO Hourly Digest status");
    }
  };

  const loadDigestStateBestEffort = async () => {
    try {
      const [nextDigestConfig, nextDigestGate] = await Promise.all([
        invokeCommand<CeoDigestConfig>("get_ceo_digest_config"),
        invokeCommand<CeoDigestGateStatus>("get_ceo_digest_gate_status"),
      ]);
      applyDigestConfig(nextDigestConfig);
      setDigestGateStatus(nextDigestGate);
    } catch {
      setDigestConfig(DEFAULT_DIGEST_CONFIG);
      setTelegramDeliveryChatId("");
      setDigestGateStatus({ ready: false, missing: ["digest_enabled"] });
      setStatusMessage("Unable to load CEO Hourly Digest settings");
    }
  };

  const refreshGateStatus = async () => {
    const nextGate = await invokeCommand<CeoTelegramGateStatus>("get_ceo_telegram_gate_status");
    setGateStatus(nextGate);
    await refreshDigestGateStatusBestEffort();
  };

  const refreshGateStatusBestEffort = async () => {
    try {
      await refreshGateStatus();
    } catch {
      toast.error("Unable to refresh CEO Telegram Chat status");
    }
  };

  const refreshCredentialStateAndGate = async () => {
    const [nextConfig, nextGate] = await Promise.all([
      invokeCommand<CeoTelegramConfig>("get_ceo_telegram_config"),
      invokeCommand<CeoTelegramGateStatus>("get_ceo_telegram_gate_status"),
    ]);
    setConfig((current) => ({
      ...current,
      telegram_bot_token_present: nextConfig.telegram_bot_token_present,
      openai_api_key_present: nextConfig.openai_api_key_present,
      last_update_id: nextConfig.last_update_id,
    }));
    setGateStatus(nextGate);
    setLoadError(null);
    await refreshDigestCredentialStateAndGateBestEffort();
  };

  const refreshCredentialStateAndGateBestEffort = async () => {
    try {
      await refreshCredentialStateAndGate();
    } catch {
      toast.error("Unable to refresh CEO Telegram Chat status");
    }
  };

  const loadState = async () => {
    const [nextCloudOptIn, nextConfig, nextGate] = await Promise.all([
      invokeCommand<boolean>("get_ceo_cloud_data_opt_in"),
      invokeCommand<CeoTelegramConfig>("get_ceo_telegram_config"),
      invokeCommand<CeoTelegramGateStatus>("get_ceo_telegram_gate_status"),
    ]);
    setCloudOptIn(nextCloudOptIn);
    applyConfig(nextConfig);
    setGateStatus(nextGate);
    setLoadError(null);
    await loadDigestStateBestEffort();
  };

  useEffect(() => {
    loadState()
      .catch(() => {
        setLoadError("Unable to load CEO Telegram Chat settings");
      })
      .finally(() => setLoading(false));
  }, []);

  const handleCloudOptInToggle = async (nextValue: boolean) => {
    const previous = cloudOptIn;
    setSaving("cloud-opt-in");
    setCloudOptIn(nextValue);
    setStatusMessage(null);

    try {
      await invokeWriteCommand("set_ceo_cloud_data_opt_in", { enabled: nextValue });
      const message = nextValue
        ? "CEO cloud-data opt-in enabled"
        : "CEO cloud-data opt-in revoked";
      setStatusMessage(message);
      toast.success(message);
      await refreshGateStatusBestEffort();
    } catch {
      setCloudOptIn(previous);
      setStatusMessage("Unable to update CEO cloud-data opt-in");
      toast.error("Unable to update CEO cloud-data opt-in");
    } finally {
      setSaving(null);
    }
  };

  const handleRuntimeToggle = (nextValue: boolean) => {
    setConfig((current) => ({ ...current, runtime_enabled: nextValue }));
    setStatusMessage(null);
  };

  const handleDigestToggle = (nextValue: boolean) => {
    setDigestConfig((current) => ({ ...current, digest_enabled: nextValue }));
    setStatusMessage(null);
  };

  const handleSaveConfig = async () => {
    const previous = config;
    setSaving("config");
    setStatusMessage(null);

    try {
      const nextConfig = await invokeWriteCommand<CeoTelegramConfig>("set_ceo_telegram_config", {
        runtimeEnabled: config.runtime_enabled,
        telegramUserId: telegramUserId.trim() || null,
        openaiModel,
      });
      applyConfig(nextConfig);
      setStatusMessage("CEO Telegram Chat config saved");
      toast.success("CEO Telegram Chat config saved");
      await refreshGateStatusBestEffort();
    } catch {
      applyConfig(previous);
      setStatusMessage("Unable to save CEO Telegram Chat config");
      toast.error("Unable to save CEO Telegram Chat config");
    } finally {
      setSaving(null);
    }
  };

  const handleSaveDigestConfig = async () => {
    const previous = digestConfig;
    const previousTelegramDeliveryChatId = telegramDeliveryChatId;
    setSaving("digest-config");
    setStatusMessage(null);

    const deliveryChatId = telegramDeliveryChatId.trim()
      ? Number(telegramDeliveryChatId.trim())
      : null;

    try {
      const nextConfig = await invokeWriteCommand<CeoDigestConfig>("set_ceo_digest_config", {
        digestEnabled: digestConfig.digest_enabled,
        telegramDeliveryChatId: deliveryChatId,
      });
      applyDigestConfig(nextConfig);
      setStatusMessage("CEO Hourly Digest config saved");
      toast.success("CEO Hourly Digest config saved");
      await refreshDigestGateStatusBestEffort();
    } catch {
      setDigestConfig(previous);
      setTelegramDeliveryChatId(previousTelegramDeliveryChatId);
      setStatusMessage("Unable to save CEO Hourly Digest config");
      toast.error("Unable to save CEO Hourly Digest config");
    } finally {
      setSaving(null);
    }
  };

  const handleSetTelegramToken = async () => {
    const previous = config;
    setSaving("telegram-token");
    setConfig((current) => ({ ...current, telegram_bot_token_present: true }));
    setStatusMessage(null);

    try {
      await invokeWriteCommand("set_ceo_telegram_bot_token", { token: telegramBotToken });
      setTelegramBotToken("");
      setStatusMessage("Telegram bot token saved");
      toast.success("Telegram bot token saved");
      await refreshCredentialStateAndGateBestEffort();
    } catch {
      setConfig(previous);
      setStatusMessage("Unable to save Telegram bot token");
      toast.error("Unable to save Telegram bot token");
    } finally {
      setSaving(null);
    }
  };

  const handleClearTelegramToken = async () => {
    const previous = config;
    setSaving("telegram-token");
    setConfig((current) => ({ ...current, telegram_bot_token_present: false }));
    setStatusMessage(null);

    try {
      await invokeWriteCommand("clear_ceo_telegram_bot_token");
      setStatusMessage("Telegram bot token cleared");
      toast.success("Telegram bot token cleared");
      await refreshCredentialStateAndGateBestEffort();
    } catch {
      setConfig(previous);
      setStatusMessage("Unable to clear Telegram bot token");
      toast.error("Unable to clear Telegram bot token");
    } finally {
      setSaving(null);
    }
  };

  const handleSetOpenAiKey = async () => {
    const previous = config;
    setSaving("openai-key");
    setConfig((current) => ({ ...current, openai_api_key_present: true }));
    setStatusMessage(null);

    try {
      await invokeWriteCommand("set_ceo_openai_api_key", { apiKey: openaiApiKey });
      setOpenaiApiKey("");
      setStatusMessage("OpenAI API key saved");
      toast.success("OpenAI API key saved");
      await refreshCredentialStateAndGateBestEffort();
    } catch {
      setConfig(previous);
      setStatusMessage("Unable to save OpenAI API key");
      toast.error("Unable to save OpenAI API key");
    } finally {
      setSaving(null);
    }
  };

  const handleClearOpenAiKey = async () => {
    const previous = config;
    setSaving("openai-key");
    setConfig((current) => ({ ...current, openai_api_key_present: false }));
    setStatusMessage(null);

    try {
      await invokeWriteCommand("clear_ceo_openai_api_key");
      setStatusMessage("OpenAI API key cleared");
      toast.success("OpenAI API key cleared");
      await refreshCredentialStateAndGateBestEffort();
    } catch {
      setConfig(previous);
      setStatusMessage("Unable to clear OpenAI API key");
      toast.error("Unable to clear OpenAI API key");
    } finally {
      setSaving(null);
    }
  };

  const disabled = loading || Boolean(loadError) || saving !== null;

  return (
    <div className="max-w-3xl space-y-5">
      <div>
        <h3 className="mb-1 text-lg font-bold">CEO Telegram Chat</h3>
        <p className="text-sm text-brand-muted">
          Owner-bound Telegram access to read-only PMS answers.
        </p>
      </div>

      <section className="space-y-3 rounded-xl border border-slate-200 p-4">
        <div className="flex items-center justify-between gap-4">
          <div>
            <p className="text-sm font-semibold">Gate status</p>
            <p className={gateStatus.ready ? "text-sm text-emerald-700" : "text-sm text-amber-700"}>
              {gateStatus.ready ? "Ready" : "Not ready"}
            </p>
          </div>
          <label className="flex items-center gap-2 text-sm font-medium">
            <span>Runtime enabled</span>
            <input
              type="checkbox"
              aria-label="Runtime enabled"
              checked={config.runtime_enabled}
              disabled={disabled}
              onChange={(event) => handleRuntimeToggle(event.target.checked)}
            />
          </label>
        </div>

        <ul className="grid gap-2 text-sm sm:grid-cols-2">
          {Object.entries(GATE_LABELS).map(([key, label]) => {
            const missing = gateStatus.missing.includes(key);
            return (
              <li
                key={key}
                className={missing ? "text-amber-700" : "text-emerald-700"}
                aria-label={`${label}: ${missing ? "missing" : "ready"}`}
              >
                {label}: {missing ? "missing" : "ready"}
              </li>
            );
          })}
        </ul>
      </section>

      <section className="space-y-4 rounded-xl border border-slate-200 p-4">
        <div className="flex items-center justify-between gap-4">
          <div>
            <h3 className="text-sm font-semibold">CEO Hourly Digest</h3>
            <p
              className={
                digestGateStatus.ready ? "text-sm text-emerald-700" : "text-sm text-amber-700"
              }
            >
              {digestGateStatus.ready ? "Ready" : "Not ready"}
            </p>
          </div>
          <label className="flex items-center gap-2 text-sm font-medium">
            <span>Digest enabled</span>
            <input
              type="checkbox"
              aria-label="CEO Hourly Digest enabled"
              checked={digestConfig.digest_enabled}
              disabled={disabled}
              onChange={(event) => handleDigestToggle(event.target.checked)}
            />
          </label>
        </div>

        <ul className="grid gap-2 text-sm sm:grid-cols-2">
          {Object.entries(DIGEST_GATE_LABELS).map(([key, label]) => {
            const missing = digestGateStatus.missing.includes(key);
            return (
              <li
                key={key}
                className={missing ? "text-amber-700" : "text-emerald-700"}
                aria-label={`${label}: ${missing ? "missing" : "ready"}`}
              >
                {label}: {missing ? "missing" : "ready"}
              </li>
            );
          })}
        </ul>

        <label className="block space-y-1 text-sm font-medium">
          <span>Telegram delivery chat ID</span>
          <input
            aria-label="Telegram delivery chat ID"
            inputMode="numeric"
            pattern="-?[0-9]*"
            value={telegramDeliveryChatId}
            disabled={disabled}
            onChange={(event) =>
              setTelegramDeliveryChatId(event.target.value.replace(/[^\d-]/g, "").replace(/(?!^)-/g, ""))
            }
            className="h-10 w-full rounded-xl border border-slate-200 bg-slate-50 px-3 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200 disabled:opacity-60"
          />
        </label>

        <button
          type="button"
          disabled={disabled}
          onClick={() => void handleSaveDigestConfig()}
          className="rounded-xl bg-brand-primary px-4 py-2 text-sm font-medium text-white disabled:opacity-60"
        >
          Save digest config
        </button>
      </section>

      <label className="flex items-center justify-between rounded-xl border border-slate-200 p-4">
        <div className="pr-4 space-y-1">
          <p className="text-sm font-medium">Allow CEO cloud-data processing</p>
          <p className="text-xs text-brand-muted">Required for cloud AI calls with PMS data.</p>
        </div>
        <input
          type="checkbox"
          aria-label="Allow CEO cloud-data processing"
          checked={cloudOptIn}
          disabled={disabled}
          onChange={(event) => void handleCloudOptInToggle(event.target.checked)}
        />
      </label>

      <section className="space-y-4 rounded-xl border border-slate-200 p-4">
        <div className="grid gap-4 sm:grid-cols-2">
          <label className="space-y-1 text-sm font-medium">
            <span>Telegram owner ID</span>
            <input
              aria-label="Telegram owner ID"
              inputMode="numeric"
              pattern="[0-9]*"
              value={telegramUserId}
              disabled={disabled}
              onChange={(event) => setTelegramUserId(event.target.value.replace(/\D/g, ""))}
              className="h-10 w-full rounded-xl border border-slate-200 bg-slate-50 px-3 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200 disabled:opacity-60"
            />
          </label>

          <label className="space-y-1 text-sm font-medium">
            <span>OpenAI model</span>
            <select
              aria-label="OpenAI model"
              value={openaiModel}
              disabled={disabled}
              onChange={(event) => setOpenaiModel(event.target.value)}
              className="h-10 w-full rounded-xl border border-slate-200 bg-slate-50 px-3 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200 disabled:opacity-60"
            >
              <option value="gpt-5">gpt-5</option>
            </select>
          </label>
        </div>

        <button
          type="button"
          disabled={disabled}
          onClick={() => void handleSaveConfig()}
          className="rounded-xl bg-brand-primary px-4 py-2 text-sm font-medium text-white disabled:opacity-60"
        >
          Save Telegram config
        </button>
      </section>

      <section className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-3 rounded-xl border border-slate-200 p-4">
          <div>
            <p className="text-sm font-semibold">Telegram bot token</p>
            <p className="text-xs text-brand-muted">
              {config.telegram_bot_token_present ? "Saved" : "Missing"}
            </p>
          </div>
          <input
            type="password"
            aria-label="Telegram bot token"
            value={telegramBotToken}
            disabled={disabled}
            onChange={(event) => setTelegramBotToken(event.target.value)}
            className="h-10 w-full rounded-xl border border-slate-200 bg-slate-50 px-3 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200 disabled:opacity-60"
          />
          <div className="flex gap-2">
            <button
              type="button"
              disabled={disabled || telegramBotToken.trim().length === 0}
              onClick={() => void handleSetTelegramToken()}
              className="rounded-xl bg-brand-primary px-3 py-2 text-sm font-medium text-white disabled:opacity-60"
            >
              Save token
            </button>
            <button
              type="button"
              disabled={disabled || !config.telegram_bot_token_present}
              onClick={() => void handleClearTelegramToken()}
              className="rounded-xl border border-slate-300 px-3 py-2 text-sm font-medium disabled:opacity-60"
            >
              Clear token
            </button>
          </div>
        </div>

        <div className="space-y-3 rounded-xl border border-slate-200 p-4">
          <div>
            <p className="text-sm font-semibold">OpenAI API key</p>
            <p className="text-xs text-brand-muted">
              {config.openai_api_key_present ? "Saved" : "Missing"}
            </p>
          </div>
          <input
            type="password"
            aria-label="OpenAI API key"
            value={openaiApiKey}
            disabled={disabled}
            onChange={(event) => setOpenaiApiKey(event.target.value)}
            className="h-10 w-full rounded-xl border border-slate-200 bg-slate-50 px-3 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200 disabled:opacity-60"
          />
          <div className="flex gap-2">
            <button
              type="button"
              disabled={disabled || openaiApiKey.trim().length === 0}
              onClick={() => void handleSetOpenAiKey()}
              className="rounded-xl bg-brand-primary px-3 py-2 text-sm font-medium text-white disabled:opacity-60"
            >
              Save key
            </button>
            <button
              type="button"
              disabled={disabled || !config.openai_api_key_present}
              onClick={() => void handleClearOpenAiKey()}
              className="rounded-xl border border-slate-300 px-3 py-2 text-sm font-medium disabled:opacity-60"
            >
              Clear key
            </button>
          </div>
        </div>
      </section>

      {loadError && <p className="text-sm text-red-500">{loadError}</p>}
      {statusMessage && <p className="text-sm text-brand-text">{statusMessage}</p>}
    </div>
  );
}
