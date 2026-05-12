import { useEffect, useState } from "react";

import { invokeCommand } from "@/lib/invokeCommand";

const DEFAULT_ENDPOINT = "http://127.0.0.1:8080/v1/chat/completions";
const DEFAULT_MODEL = "capyinn-gemma4-e2b-q5km";
const ENDPOINT_STORAGE_KEY = "capyinn.localReceptionist.endpoint";
const MODEL_STORAGE_KEY = "capyinn.localReceptionist.model";
const UNREACHABLE_MESSAGE = "Local provider is not reachable. Start llama-server and try again.";
const MAX_ENDPOINT_CHARS = 2048;
const MAX_MODEL_CHARS = 128;
const MAX_MESSAGE_CHARS = 2000;

type LocalReceptionistChatResponse = {
  answer: string;
  provider: "local";
  model: string;
};

type LocalFieldErrors = {
  endpoint?: string;
  model?: string;
  message?: string;
};

function readLocalStorage(key: string, fallback: string) {
  try {
    return window.localStorage.getItem(key) || fallback;
  } catch {
    return fallback;
  }
}

function writeLocalStorage(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // Ignore storage failures; the demo can still run with component state.
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function collectErrorMessages(error: unknown, messages: string[] = []): string[] {
  if (typeof error === "string") {
    messages.push(error);
    return messages;
  }

  if (error instanceof Error) {
    messages.push(error.message);
    if ("cause" in error) {
      collectErrorMessages((error as Error & { cause?: unknown }).cause, messages);
    }
    return messages;
  }

  if (isRecord(error)) {
    if (typeof error.message === "string") {
      messages.push(error.message);
    }
    if ("cause" in error) {
      collectErrorMessages(error.cause, messages);
    }
  }

  return messages;
}

function localErrorMessage(error: unknown) {
  const message = collectErrorMessages(error).join(" ").toLowerCase();
  if (message.includes("timed out") || message.includes("timeout")) {
    return "Local provider timed out. Try a shorter question or restart llama-server.";
  }
  if (message.includes("rejected")) {
    return "Local provider rejected the request. Check the endpoint and model name.";
  }
  if (message.includes("too large")) {
    return "Local provider response was too large. Try a shorter answer.";
  }
  if (message.includes("unsupported")) {
    return "Local provider returned an unsupported response.";
  }
  if (message.includes("invalid")) {
    return "Please check the local endpoint, model name, and message.";
  }
  return UNREACHABLE_MESSAGE;
}

function validateLocalEndpoint(value: string) {
  const endpoint = value.trim();
  if (!endpoint) {
    return "Endpoint is required.";
  }
  if (endpoint.length > MAX_ENDPOINT_CHARS) {
    return "Endpoint is too long.";
  }
  try {
    const parsed = new URL(endpoint);
    if (
      parsed.protocol !== "http:" ||
      (parsed.hostname !== "127.0.0.1" && parsed.hostname !== "localhost")
    ) {
      return "Endpoint must be http://127.0.0.1 or http://localhost.";
    }
  } catch {
    return "Endpoint must be a valid local HTTP URL.";
  }
  return "";
}

function validateModelName(value: string) {
  const model = value.trim();
  if (!model) {
    return "Model name is required.";
  }
  if (model.length > MAX_MODEL_CHARS || /[\u0000-\u001f\u007f]/.test(model)) {
    return "Model name is invalid.";
  }
  return "";
}

function validateMessage(value: string) {
  const text = value.trim();
  if (!text) {
    return "Message is required.";
  }
  if (text.length > MAX_MESSAGE_CHARS) {
    return "Message is too long.";
  }
  return "";
}

export default function LocalReceptionistDemo() {
  const [endpoint, setEndpoint] = useState(() =>
    readLocalStorage(ENDPOINT_STORAGE_KEY, DEFAULT_ENDPOINT),
  );
  const [model, setModel] = useState(() => readLocalStorage(MODEL_STORAGE_KEY, DEFAULT_MODEL));
  const [message, setMessage] = useState("");
  const [answer, setAnswer] = useState("");
  const [error, setError] = useState("");
  const [fieldErrors, setFieldErrors] = useState<LocalFieldErrors>({});
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    writeLocalStorage(ENDPOINT_STORAGE_KEY, endpoint);
  }, [endpoint]);

  useEffect(() => {
    writeLocalStorage(MODEL_STORAGE_KEY, model);
  }, [model]);

  const canSubmit = message.trim().length > 0 && !loading;

  const clearFieldError = (field: keyof LocalFieldErrors) => {
    setFieldErrors((current) =>
      current[field] === undefined ? current : { ...current, [field]: undefined },
    );
  };

  const handleAsk = async () => {
    if (!canSubmit) {
      return;
    }

    const nextFieldErrors: LocalFieldErrors = {
      endpoint: validateLocalEndpoint(endpoint) || undefined,
      model: validateModelName(model) || undefined,
      message: validateMessage(message) || undefined,
    };
    if (nextFieldErrors.endpoint || nextFieldErrors.model || nextFieldErrors.message) {
      setAnswer("");
      setError("");
      setFieldErrors(nextFieldErrors);
      return;
    }

    setLoading(true);
    setAnswer("");
    setError("");
    setFieldErrors({});
    try {
      const result = await invokeCommand<LocalReceptionistChatResponse>(
        "local_receptionist_chat",
        {
          endpoint: endpoint.trim(),
          model: model.trim(),
          message: message.trim(),
        },
      );
      setAnswer(result.answer);
    } catch (caught) {
      setError(localErrorMessage(caught));
    } finally {
      setLoading(false);
    }
  };

  return (
    <section className="space-y-4 rounded-xl border border-slate-200 p-4">
      <div>
        <h3 className="text-sm font-semibold">Local Receptionist Demo</h3>
        <p className="text-xs text-brand-muted">
          Runs locally through a provider on 127.0.0.1 or localhost. Uses hotel info and pricing
          context only. No PMS writes.
        </p>
      </div>

      <div className="grid gap-4 sm:grid-cols-2">
        <label className="space-y-1 text-sm font-medium">
          <span>Local provider endpoint</span>
          <input
            aria-label="Local provider endpoint"
            aria-describedby={
              fieldErrors.endpoint ? "local-receptionist-endpoint-error" : undefined
            }
            aria-invalid={Boolean(fieldErrors.endpoint)}
            value={endpoint}
            disabled={loading}
            onChange={(event) => {
              setEndpoint(event.target.value);
              clearFieldError("endpoint");
            }}
            className="h-10 w-full rounded-xl border border-slate-200 bg-slate-50 px-3 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200 disabled:opacity-60"
          />
          {fieldErrors.endpoint && (
            <p id="local-receptionist-endpoint-error" className="text-xs text-red-500">
              {fieldErrors.endpoint}
            </p>
          )}
        </label>

        <label className="space-y-1 text-sm font-medium">
          <span>Local model name</span>
          <input
            aria-label="Local model name"
            aria-describedby={fieldErrors.model ? "local-receptionist-model-error" : undefined}
            aria-invalid={Boolean(fieldErrors.model)}
            value={model}
            disabled={loading}
            onChange={(event) => {
              setModel(event.target.value);
              clearFieldError("model");
            }}
            className="h-10 w-full rounded-xl border border-slate-200 bg-slate-50 px-3 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200 disabled:opacity-60"
          />
          {fieldErrors.model && (
            <p id="local-receptionist-model-error" className="text-xs text-red-500">
              {fieldErrors.model}
            </p>
          )}
        </label>
      </div>

      <label className="block space-y-1 text-sm font-medium">
        <span>Receptionist message</span>
        <textarea
          aria-label="Receptionist message"
          aria-describedby={fieldErrors.message ? "local-receptionist-message-error" : undefined}
          aria-invalid={Boolean(fieldErrors.message)}
          value={message}
          disabled={loading}
          rows={3}
          onChange={(event) => {
            setMessage(event.target.value);
            clearFieldError("message");
          }}
          className="w-full rounded-xl border border-slate-200 bg-slate-50 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200 disabled:opacity-60"
        />
        {fieldErrors.message && (
          <p id="local-receptionist-message-error" className="text-xs text-red-500">
            {fieldErrors.message}
          </p>
        )}
      </label>

      <button
        type="button"
        disabled={!canSubmit}
        onClick={() => void handleAsk()}
        className="rounded-xl bg-brand-primary px-4 py-2 text-sm font-medium text-white disabled:opacity-60"
      >
        {loading ? "Asking local Gemma..." : "Ask local Gemma"}
      </button>

      {answer && (
        <div className="rounded-xl border border-emerald-200 bg-emerald-50 p-3 text-sm text-emerald-900">
          {answer}
        </div>
      )}
      {error && <p className="text-sm text-red-500">{error}</p>}
    </section>
  );
}
