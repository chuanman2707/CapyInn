import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((promiseResolve, promiseReject) => {
    resolve = promiseResolve;
    reject = promiseReject;
  });
  return { promise, resolve, reject };
}

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
    window.localStorage.clear();

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

    const ownerInput = await screen.findByLabelText("Telegram owner ID");
    const runtimeCheckbox = screen.getByRole("checkbox", { name: "Runtime enabled" });
    const saveButton = screen.getByRole("button", { name: "Save Telegram config" });

    await waitForEnabled(ownerInput);
    await waitForEnabled(runtimeCheckbox);
    await user.type(ownerInput, "abc123456");
    await user.click(runtimeCheckbox);
    expect(invoke.mock.calls.some(([command]) => command === "set_ceo_telegram_config")).toBe(
      false,
    );

    await waitForEnabled(saveButton);
    await user.click(saveButton);

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

    const digestEnabled = await screen.findByRole("checkbox", { name: "CEO Hourly Digest enabled" });
    const deliveryChatInput = screen.getByLabelText("Telegram delivery chat ID");
    const saveButton = screen.getByRole("button", { name: "Save digest config" });

    await waitForEnabled(digestEnabled);
    await waitForEnabled(deliveryChatInput);
    await user.click(digestEnabled);
    await user.type(deliveryChatInput, "55");
    await waitForEnabled(saveButton);
    await user.click(saveButton);

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

    const deliveryChatInput = await screen.findByLabelText("Telegram delivery chat ID");
    const saveButton = screen.getByRole("button", { name: "Save digest config" });

    await waitForEnabled(deliveryChatInput);
    await user.type(deliveryChatInput, "-10055");
    await waitForEnabled(saveButton);
    await user.click(saveButton);

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
    const deliveryChatInput = screen.getByLabelText("Telegram delivery chat ID");
    const saveButton = screen.getByRole("button", { name: "Save digest config" });

    await waitForEnabled(digestEnabled);
    await waitForEnabled(deliveryChatInput);
    await user.click(digestEnabled);
    await user.type(deliveryChatInput, "55");
    await waitForEnabled(saveButton);
    await user.click(saveButton);

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

  it("renders the local receptionist demo and persists endpoint and model", async () => {
    const user = userEvent.setup();
    mockInitialState();

    render(<CeoAgentSection />);

    expect(await screen.findByText("Local Receptionist Demo")).toBeInTheDocument();
    const endpoint = screen.getByLabelText("Local provider endpoint");
    const model = screen.getByLabelText("Local model name");

    await user.clear(endpoint);
    await user.type(endpoint, "http://localhost:8081/v1/chat/completions");
    await user.clear(model);
    await user.type(model, "capyinn-local-test");

    expect(window.localStorage.getItem("capyinn.localReceptionist.endpoint")).toBe(
      "http://localhost:8081/v1/chat/completions",
    );
    expect(window.localStorage.getItem("capyinn.localReceptionist.model")).toBe(
      "capyinn-local-test",
    );
  });

  it("calls local_receptionist_chat and shows the answer", async () => {
    const user = userEvent.setup();
    mockInitialState();
    setMockResponses({
      local_receptionist_chat: (args) => ({
        answer: `Answer for ${(args?.message as string) ?? ""}`,
        provider: "local",
        model: (args?.model as string) ?? "capyinn-gemma4-e2b-q5km",
      }),
    });

    render(<CeoAgentSection />);

    await user.type(
      await screen.findByLabelText("Receptionist message"),
      "Do you have hourly rooms?",
    );
    await user.click(screen.getByRole("button", { name: "Ask local Gemma" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith(
        "local_receptionist_chat",
        expect.objectContaining({
          endpoint: "http://127.0.0.1:8080/v1/chat/completions",
          model: "capyinn-gemma4-e2b-q5km",
          message: "Do you have hourly rooms?",
        }),
      );
    });
    expect(await screen.findByText("Answer for Do you have hourly rooms?")).toBeInTheDocument();
  });

  it("disables local receptionist submit for blank messages", async () => {
    mockInitialState();

    render(<CeoAgentSection />);

    expect(await screen.findByRole("button", { name: "Ask local Gemma" })).toBeDisabled();
  });

  it("shows field validation and does not invoke for remote endpoints", async () => {
    const user = userEvent.setup();
    mockInitialState();

    render(<CeoAgentSection />);

    await user.clear(await screen.findByLabelText("Local provider endpoint"));
    await user.type(
      screen.getByLabelText("Local provider endpoint"),
      "https://example.com/v1/chat/completions",
    );
    await user.type(await screen.findByLabelText("Receptionist message"), "Hello");
    await user.click(screen.getByRole("button", { name: "Ask local Gemma" }));

    expect(
      await screen.findByText("Endpoint must be http://127.0.0.1 or http://localhost."),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Local provider endpoint")).toHaveAttribute(
      "aria-describedby",
      "local-receptionist-endpoint-error",
    );
    expect(screen.getByLabelText("Local provider endpoint")).toHaveAttribute("aria-invalid", "true");
    expect(invoke).not.toHaveBeenCalledWith("local_receptionist_chat", expect.anything());
  });

  it("shows endpoint length validation on the endpoint field", async () => {
    const user = userEvent.setup();
    mockInitialState();

    render(<CeoAgentSection />);

    const endpoint = await screen.findByLabelText("Local provider endpoint");
    fireEvent.change(endpoint, {
      target: { value: `http://127.0.0.1/${"a".repeat(2048)}` },
    });
    await user.type(await screen.findByLabelText("Receptionist message"), "Hello");
    await user.click(screen.getByRole("button", { name: "Ask local Gemma" }));

    expect(await screen.findByText("Endpoint is too long.")).toBeInTheDocument();
    expect(endpoint).toHaveAttribute("aria-describedby", "local-receptionist-endpoint-error");
    expect(endpoint).toHaveAttribute("aria-invalid", "true");
    expect(invoke).not.toHaveBeenCalledWith("local_receptionist_chat", expect.anything());
  });

  it("shows field validation and does not invoke for blank model names", async () => {
    const user = userEvent.setup();
    mockInitialState();

    render(<CeoAgentSection />);

    await user.clear(await screen.findByLabelText("Local model name"));
    await user.type(await screen.findByLabelText("Receptionist message"), "Hello");
    await user.click(screen.getByRole("button", { name: "Ask local Gemma" }));

    expect(await screen.findByText("Model name is required.")).toBeInTheDocument();
    expect(screen.getByLabelText("Local model name")).toHaveAttribute(
      "aria-describedby",
      "local-receptionist-model-error",
    );
    expect(screen.getByLabelText("Local model name")).toHaveAttribute("aria-invalid", "true");
    expect(invoke).not.toHaveBeenCalledWith("local_receptionist_chat", expect.anything());
  });

  it("shows message validation on the message field", async () => {
    const user = userEvent.setup();
    mockInitialState();

    render(<CeoAgentSection />);

    const message = await screen.findByLabelText("Receptionist message");
    fireEvent.change(message, { target: { value: "x".repeat(2001) } });
    await user.click(screen.getByRole("button", { name: "Ask local Gemma" }));

    expect(await screen.findByText("Message is too long.")).toBeInTheDocument();
    expect(message).toHaveAttribute("aria-describedby", "local-receptionist-message-error");
    expect(message).toHaveAttribute("aria-invalid", "true");
    expect(invoke).not.toHaveBeenCalledWith("local_receptionist_chat", expect.anything());
  });

  it("disables local receptionist fields while a request is in flight", async () => {
    const user = userEvent.setup();
    const pending = deferred<{ answer: string; provider: "local"; model: string }>();
    mockInitialState();
    setMockResponses({
      local_receptionist_chat: () => pending.promise,
    });

    render(<CeoAgentSection />);

    const endpoint = await screen.findByLabelText("Local provider endpoint");
    const model = screen.getByLabelText("Local model name");
    const message = screen.getByLabelText("Receptionist message");
    await user.type(message, "Hello");
    await user.click(screen.getByRole("button", { name: "Ask local Gemma" }));

    expect(await screen.findByRole("button", { name: "Asking local Gemma..." })).toBeDisabled();
    expect(endpoint).toBeDisabled();
    expect(model).toBeDisabled();
    expect(message).toBeDisabled();

    pending.resolve({
      answer: "Local answer",
      provider: "local",
      model: "capyinn-gemma4-e2b-q5km",
    });

    expect(await screen.findByText("Local answer")).toBeInTheDocument();
  });

  it.each([
    [
      "request timed out after 60 seconds with raw endpoint details",
      "Local provider timed out. Try a shorter question or restart llama-server.",
    ],
    [
      "provider rejected request with raw status 500",
      "Local provider rejected the request. Check the endpoint and model name.",
    ],
    [
      "provider response too large with raw byte count",
      "Local provider response was too large. Try a shorter answer.",
    ],
    [
      "unsupported provider response with raw schema details",
      "Local provider returned an unsupported response.",
    ],
    [
      "invalid request with raw validation details",
      "Please check the local endpoint, model name, and message.",
    ],
  ])("shows a short local provider error for %s", async (rawError, expectedMessage) => {
    const user = userEvent.setup();
    mockInitialState();
    setMockResponses({
      local_receptionist_chat: () => {
        throw new Error(rawError);
      },
    });

    render(<CeoAgentSection />);

    await user.type(await screen.findByLabelText("Receptionist message"), "Hello");
    await user.click(screen.getByRole("button", { name: "Ask local Gemma" }));

    expect(await screen.findByText(expectedMessage)).toBeInTheDocument();
    expect(screen.queryByText(/raw/i)).not.toBeInTheDocument();
  });

  it("shows a short local provider error without raw details", async () => {
    const user = userEvent.setup();
    mockInitialState();
    setMockResponses({
      local_receptionist_chat: () => {
        throw new Error("raw tcp connect ECONNREFUSED with internal details");
      },
    });

    render(<CeoAgentSection />);

    await user.type(await screen.findByLabelText("Receptionist message"), "Hello");
    await user.click(screen.getByRole("button", { name: "Ask local Gemma" }));

    expect(
      await screen.findByText("Local provider is not reachable. Start llama-server and try again."),
    ).toBeInTheDocument();
    expect(screen.queryByText(/ECONNREFUSED/)).not.toBeInTheDocument();
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
