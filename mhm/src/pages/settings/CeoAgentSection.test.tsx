import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { clearMockResponses, invoke, setMockResponses } from "@/__mocks__/tauri-core";
import { useAuthStore } from "@/stores/useAuthStore";
import CeoAgentSection from "./CeoAgentSection";
import SettingsPage from "./index";

const { toastSuccess, toastError } = vi.hoisted(() => ({
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock("sonner", () => ({
  toast: {
    success: toastSuccess,
    error: toastError,
  },
}));

type CeoTelegramConfigFixture = {
  runtime_enabled: boolean;
  telegram_user_id: string | null;
  telegram_bot_token_present: boolean;
  openai_api_key_present: boolean;
  openai_model: string;
  last_update_id: number | null;
};

type CeoTelegramGateFixture = {
  ready: boolean;
  missing: string[];
};

type CeoDigestConfigFixture = {
  digest_enabled: boolean;
  telegram_user_id: string | null;
  telegram_delivery_chat_id: number | null;
  telegram_bot_token_present: boolean;
  openai_api_key_present: boolean;
  openai_model: string;
};

type CeoDigestGateFixture = {
  ready: boolean;
  missing: string[];
};

const disabledConfig: CeoTelegramConfigFixture = {
  runtime_enabled: false,
  telegram_user_id: null,
  telegram_bot_token_present: false,
  openai_api_key_present: false,
  openai_model: "gpt-5",
  last_update_id: null,
};

const readyConfig: CeoTelegramConfigFixture = {
  runtime_enabled: true,
  telegram_user_id: "123456",
  telegram_bot_token_present: true,
  openai_api_key_present: true,
  openai_model: "gpt-5",
  last_update_id: 91,
};

const disabledDigestConfig: CeoDigestConfigFixture = {
  digest_enabled: false,
  telegram_user_id: null,
  telegram_delivery_chat_id: null,
  telegram_bot_token_present: false,
  openai_api_key_present: false,
  openai_model: "gpt-5",
};

function mockInitialState(
  config: CeoTelegramConfigFixture = disabledConfig,
  gate: CeoTelegramGateFixture = {
    ready: false,
    missing: ["runtime_enabled", "telegram_owner_binding", "telegram_bot_token", "open_ai_api_key"],
  },
  digestConfig: CeoDigestConfigFixture = disabledDigestConfig,
  digestGate: CeoDigestGateFixture = {
    ready: false,
    missing: ["digest_enabled", "telegram_delivery_chat_id"],
  },
) {
  setMockResponses({
    get_ceo_cloud_data_opt_in: () => true,
    get_ceo_telegram_config: () => config,
    get_ceo_telegram_gate_status: () => gate,
    get_ceo_digest_config: () => digestConfig,
    get_ceo_digest_gate_status: () => digestGate,
    set_ceo_cloud_data_opt_in: () => undefined,
    set_ceo_telegram_config: (args) => ({
      ...config,
      runtime_enabled: Boolean(args?.runtimeEnabled),
      telegram_user_id: (args?.telegramUserId as string | null) ?? null,
      openai_model: (args?.openaiModel as string) ?? "gpt-5",
    }),
    set_ceo_digest_config: (args) => ({
      ...disabledDigestConfig,
      digest_enabled: Boolean(args?.digestEnabled),
      telegram_delivery_chat_id: (args?.telegramDeliveryChatId as number | null) ?? null,
    }),
    set_ceo_telegram_bot_token: () => undefined,
    clear_ceo_telegram_bot_token: () => undefined,
    set_ceo_openai_api_key: () => undefined,
    clear_ceo_openai_api_key: () => undefined,
  });
}

async function waitForEnabled(element: HTMLElement) {
  await waitFor(() => expect(element).toBeEnabled());
}

describe("CeoAgentSection", () => {
  beforeEach(() => {
    clearMockResponses();
    invoke.mockClear();
    toastSuccess.mockReset();
    toastError.mockReset();

    useAuthStore.setState({
      user: { id: "u1", name: "Admin", role: "admin", active: true, created_at: "" },
      isAuthenticated: true,
      loading: false,
      error: null,
    });
  });

  it("renders CEO Telegram Chat gate status", async () => {
    mockInitialState();

    render(<CeoAgentSection />);

    expect(await screen.findByText("CEO Telegram Chat")).toBeInTheDocument();
    expect(await screen.findByLabelText("Telegram owner binding: missing")).toBeInTheDocument();
    expect(await screen.findByLabelText("OpenAI API key: missing")).toBeInTheDocument();
  });

  it("renders separate CEO Hourly Digest gate status", async () => {
    mockInitialState();
    render(<CeoAgentSection />);

    expect(await screen.findByText("CEO Hourly Digest")).toBeInTheDocument();
    expect(await screen.findByLabelText("Telegram delivery chat ID: missing")).toBeInTheDocument();
  });

  it("shows missing digest OpenAI model requirement", async () => {
    mockInitialState();
    setMockResponses({
      get_ceo_digest_gate_status: () => ({
        ready: false,
        missing: ["open_ai_model"],
      }),
    });

    render(<CeoAgentSection />);

    expect(await screen.findByLabelText("OpenAI model: missing")).toBeInTheDocument();
  });

  it("allows an admin to toggle opt-in on via an idempotent write command", async () => {
    const user = userEvent.setup();
    mockInitialState();
    setMockResponses({
      get_ceo_cloud_data_opt_in: () => false,
    });

    render(<CeoAgentSection />);

    const checkbox = await screen.findByRole("checkbox", {
      name: "Allow CEO cloud-data processing",
    });
    expect(checkbox).not.toBeChecked();

    await user.click(checkbox);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith(
        "set_ceo_cloud_data_opt_in",
        expect.objectContaining({
          enabled: true,
          idempotencyKey: expect.stringMatching(/^set_ceo_cloud_data_opt_in:/),
        }),
      );
    });
    expect(toastSuccess).toHaveBeenCalledWith("CEO cloud-data opt-in enabled");
  });

  it("reverts the opt-in checkbox and shows an error toast when the update fails", async () => {
    const user = userEvent.setup();
    mockInitialState();
    setMockResponses({
      get_ceo_cloud_data_opt_in: () => false,
      set_ceo_cloud_data_opt_in: () => {
        throw new Error("boom");
      },
    });

    render(<CeoAgentSection />);

    const checkbox = await screen.findByRole("checkbox", {
      name: "Allow CEO cloud-data processing",
    });
    await user.click(checkbox);

    await waitFor(() =>
      expect(toastError).toHaveBeenCalledWith("Unable to update CEO cloud-data opt-in"),
    );
    expect(checkbox).not.toBeChecked();
  });

  it("does not roll back committed opt-in when gate refresh fails", async () => {
    const user = userEvent.setup();
    let gateCalls = 0;
    mockInitialState();
    setMockResponses({
      get_ceo_cloud_data_opt_in: () => false,
      get_ceo_telegram_gate_status: () => {
        gateCalls += 1;
        if (gateCalls === 1) {
          return {
            ready: false,
            missing: ["cloud_data_opt_in", "runtime_enabled"],
          };
        }
        throw new Error("gate refresh failed");
      },
    });

    render(<CeoAgentSection />);

    const checkbox = await screen.findByRole("checkbox", {
      name: "Allow CEO cloud-data processing",
    });
    await user.click(checkbox);

    await waitFor(() => expect(toastSuccess).toHaveBeenCalledWith("CEO cloud-data opt-in enabled"));
    expect(toastError).toHaveBeenCalledWith("Unable to refresh CEO Telegram Chat status");
    expect(checkbox).toBeChecked();
  });

  it("saves owner binding, model, and runtime flag", async () => {
    const user = userEvent.setup();
    mockInitialState();

    render(<CeoAgentSection />);

    await user.type(await screen.findByLabelText("Telegram owner ID"), "abc123456");
    await user.click(screen.getByRole("checkbox", { name: "Runtime enabled" }));
    expect(invoke.mock.calls.some(([command]) => command === "set_ceo_telegram_config")).toBe(
      false,
    );

    await user.click(screen.getByRole("button", { name: "Save Telegram config" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith(
        "set_ceo_telegram_config",
        expect.objectContaining({
          runtimeEnabled: true,
          telegramUserId: "123456",
          openaiModel: "gpt-5",
          idempotencyKey: expect.stringMatching(/^set_ceo_telegram_config:/),
        }),
      );
    });
    expect(invoke.mock.calls.filter(([command]) => command === "set_ceo_telegram_config")).toHaveLength(1);
  });

  it("saves digest toggle and delivery chat id", async () => {
    const user = userEvent.setup();
    mockInitialState();
    render(<CeoAgentSection />);

    await user.click(await screen.findByRole("checkbox", { name: "CEO Hourly Digest enabled" }));
    await user.type(screen.getByLabelText("Telegram delivery chat ID"), "55");
    await user.click(screen.getByRole("button", { name: "Save digest config" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith(
        "set_ceo_digest_config",
        expect.objectContaining({
          digestEnabled: true,
          telegramDeliveryChatId: 55,
          idempotencyKey: expect.stringMatching(/^set_ceo_digest_config:/),
        }),
      );
    });
  });

  it("saves signed digest delivery chat ids for Telegram groups", async () => {
    const user = userEvent.setup();
    mockInitialState();
    render(<CeoAgentSection />);

    await user.type(await screen.findByLabelText("Telegram delivery chat ID"), "-10055");
    await user.click(screen.getByRole("button", { name: "Save digest config" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith(
        "set_ceo_digest_config",
        expect.objectContaining({
          telegramDeliveryChatId: -10055,
          idempotencyKey: expect.stringMatching(/^set_ceo_digest_config:/),
        }),
      );
    });
    expect(screen.getByLabelText("Telegram delivery chat ID")).toHaveValue("-10055");
  });

  it("keeps saved digest config when post-save gate refresh fails", async () => {
    const user = userEvent.setup();
    let digestGateCalls = 0;
    mockInitialState();
    setMockResponses({
      get_ceo_digest_gate_status: () => {
        digestGateCalls += 1;
        if (digestGateCalls === 1) {
          return {
            ready: false,
            missing: ["digest_enabled", "telegram_delivery_chat_id"],
          };
        }
        throw new Error("digest gate refresh failed");
      },
    });

    render(<CeoAgentSection />);

    const digestEnabled = await screen.findByRole("checkbox", {
      name: "CEO Hourly Digest enabled",
    });
    await user.click(digestEnabled);
    await user.type(screen.getByLabelText("Telegram delivery chat ID"), "55");
    await user.click(screen.getByRole("button", { name: "Save digest config" }));

    await waitFor(() =>
      expect(toastSuccess).toHaveBeenCalledWith("CEO Hourly Digest config saved"),
    );
    expect(toastError).toHaveBeenCalledWith("Unable to refresh CEO Hourly Digest status");
    expect(toastError).not.toHaveBeenCalledWith("Unable to save CEO Hourly Digest config");
    expect(digestEnabled).toBeChecked();
    expect(screen.getByLabelText("Telegram delivery chat ID")).toHaveValue("55");
  });

  it("saves and clears Telegram and OpenAI secrets", async () => {
    const user = userEvent.setup();
    mockInitialState(readyConfig, { ready: true, missing: [] });

    render(<CeoAgentSection />);

    const telegramTokenInput = await screen.findByLabelText("Telegram bot token");
    await waitForEnabled(telegramTokenInput);
    await user.type(telegramTokenInput, "telegram-token");

    const saveTokenButton = screen.getByRole("button", { name: "Save token" });
    await waitForEnabled(saveTokenButton);
    await user.click(saveTokenButton);
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith(
        "set_ceo_telegram_bot_token",
        expect.objectContaining({
          token: "telegram-token",
          idempotencyKey: expect.stringMatching(/^set_ceo_telegram_bot_token:/),
        }),
      );
    });

    const clearTokenButton = screen.getByRole("button", { name: "Clear token" });
    await waitForEnabled(clearTokenButton);
    await user.click(clearTokenButton);
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith(
        "clear_ceo_telegram_bot_token",
        expect.objectContaining({
          idempotencyKey: expect.stringMatching(/^clear_ceo_telegram_bot_token:/),
        }),
      );
    });

    const openAiApiKeyInput = screen.getByLabelText("OpenAI API key");
    await waitForEnabled(openAiApiKeyInput);
    await user.type(openAiApiKeyInput, "sk-test");

    const saveKeyButton = screen.getByRole("button", { name: "Save key" });
    await waitForEnabled(saveKeyButton);
    await user.click(saveKeyButton);
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith(
        "set_ceo_openai_api_key",
        expect.objectContaining({
          apiKey: "sk-test",
          idempotencyKey: expect.stringMatching(/^set_ceo_openai_api_key:/),
        }),
      );
    });

    const clearKeyButton = screen.getByRole("button", { name: "Clear key" });
    await waitForEnabled(clearKeyButton);
    await user.click(clearKeyButton);
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith(
        "clear_ceo_openai_api_key",
        expect.objectContaining({
          idempotencyKey: expect.stringMatching(/^clear_ceo_openai_api_key:/),
        }),
      );
    });
  });

  it("preserves unsaved owner draft when saving a secret refreshes credentials", async () => {
    const user = userEvent.setup();
    mockInitialState();

    render(<CeoAgentSection />);

    const ownerInput = await screen.findByLabelText("Telegram owner ID");
    await waitForEnabled(ownerInput);
    await user.type(ownerInput, "987654");

    const telegramTokenInput = screen.getByLabelText("Telegram bot token");
    await waitForEnabled(telegramTokenInput);
    await user.type(telegramTokenInput, "telegram-token");

    const saveTokenButton = screen.getByRole("button", { name: "Save token" });
    await waitForEnabled(saveTokenButton);
    await user.click(saveTokenButton);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith(
        "set_ceo_telegram_bot_token",
        expect.objectContaining({
          token: "telegram-token",
        }),
      );
    });
    expect(ownerInput).toHaveValue("987654");
  });

  it("keeps CEO Telegram Chat settings enabled when initial digest load fails", async () => {
    mockInitialState(readyConfig, { ready: true, missing: [] });
    setMockResponses({
      get_ceo_digest_config: () => {
        throw new Error("digest config unavailable");
      },
      get_ceo_digest_gate_status: () => {
        throw new Error("digest gate unavailable");
      },
    });

    render(<CeoAgentSection />);

    const optInCheckbox = await screen.findByRole("checkbox", {
      name: "Allow CEO cloud-data processing",
    });

    expect(screen.queryByText("Unable to load CEO Telegram Chat settings")).not.toBeInTheDocument();
    expect(optInCheckbox).toBeEnabled();
    expect(screen.getByRole("checkbox", { name: "Runtime enabled" })).toBeEnabled();
  });

  it("keeps controls disabled when loading settings fails", async () => {
    setMockResponses({
      get_ceo_cloud_data_opt_in: () => {
        throw new Error("unavailable");
      },
      get_ceo_telegram_config: () => disabledConfig,
      get_ceo_telegram_gate_status: () => ({ ready: false, missing: ["runtime_enabled"] }),
    });

    render(<CeoAgentSection />);

    const checkbox = await screen.findByRole("checkbox", {
      name: "Allow CEO cloud-data processing",
    });

    await waitFor(() => {
      expect(screen.getByText("Unable to load CEO Telegram Chat settings")).toBeInTheDocument();
    });
    expect(checkbox).toBeDisabled();
  });
});

describe("SettingsPage CEO Agent nav", () => {
  beforeEach(() => {
    clearMockResponses();
    invoke.mockClear();
  });

  it("shows CEO Agent in settings nav for admins and renders the section", async () => {
    const user = userEvent.setup();
    useAuthStore.setState({
      user: { id: "u1", name: "Admin", role: "admin", active: true, created_at: "" },
      isAuthenticated: true,
      loading: false,
      error: null,
    });

    mockInitialState();

    render(<SettingsPage />);

    const navButton = screen.getByRole("button", { name: "CEO Agent" });
    await user.click(navButton);

    expect(await screen.findByText("CEO Telegram Chat")).toBeInTheDocument();
  });

  it("does not show CEO Agent in settings nav for receptionist users", () => {
    useAuthStore.setState({
      user: { id: "u2", name: "Reception", role: "receptionist", active: true, created_at: "" },
      isAuthenticated: true,
      loading: false,
      error: null,
    });

    render(<SettingsPage />);

    expect(screen.queryByRole("button", { name: "CEO Agent" })).not.toBeInTheDocument();
  });
});
