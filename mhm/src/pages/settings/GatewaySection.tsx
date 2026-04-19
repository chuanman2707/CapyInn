import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Copy, Eye, EyeOff, Key, RefreshCw, Wifi } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { APP_API_KEY_PREFIX, APP_NAME } from "@/lib/appIdentity";
import { useAuthStore } from "@/stores/useAuthStore";
import type { GatewayStatus } from "@/types";

type GatewayTool = {
  name: string;
  writable: boolean;
};

export const MCP_TOOLS: GatewayTool[] = [
  { name: "get_hotel_context", writable: false },
  { name: "check_availability", writable: false },
  { name: "get_rooms", writable: false },
  { name: "get_room_detail", writable: false },
  { name: "get_room_types", writable: false },
  { name: "get_dashboard_stats", writable: false },
  { name: "get_all_bookings", writable: false },
  { name: "get_rooms_availability", writable: false },
  { name: "get_pricing_rules", writable: false },
  { name: "get_hotel_info", writable: false },
  { name: "calculate_price", writable: false },
  { name: "get_invoice", writable: false },
  { name: "create_reservation", writable: true },
  { name: "modify_reservation", writable: true },
  { name: "cancel_reservation", writable: true },
];

export function buildHttpMcpConfig(status: GatewayStatus | null, apiKey: string | null) {
  const port = status?.port ?? "<gateway-port>";
  const key = apiKey || `${APP_API_KEY_PREFIX}...`;

  return JSON.stringify(
    {
      mcpServers: {
        capyinn: {
          transport: "streamable-http",
          url: `http://127.0.0.1:${port}/mcp`,
          headers: {
            Authorization: `Bearer ${key}`,
          },
        },
      },
    },
    null,
    2,
  );
}

export default function GatewaySection() {
  const canManageGateway = useAuthStore((state) => state.user?.role === "admin");
  const [status, setStatus] = useState<GatewayStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [apiKey, setApiKey] = useState<string | null>(null);
  const [label, setLabel] = useState("default");
  const [showKey, setShowKey] = useState(false);
  const [generating, setGenerating] = useState(false);

  const fetchStatus = () => {
    invoke<GatewayStatus>("gateway_get_status")
      .then(setStatus)
      .catch(() => setStatus({ running: false, port: null, has_api_keys: false }))
      .finally(() => setLoading(false));
  };

  useEffect(fetchStatus, []);

  const handleGenerate = async () => {
    setGenerating(true);
    try {
      const key = await invoke<string>("gateway_generate_key", { label: label || "default" });
      setApiKey(key);
      setShowKey(true);
      fetchStatus();
      toast.success("Đã tạo API key mới!");
    } catch (error) {
      toast.error(String(error) || "Lỗi tạo API key");
    } finally {
      setGenerating(false);
    }
  };

  const copyMcpConfig = () => {
    const config = buildHttpMcpConfig(status, apiKey);

    void navigator.clipboard.writeText(config);
    toast.success("Đã copy MCP Config! Dán vào AI agent config.");
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center py-16 text-brand-muted">
        <RefreshCw size={20} className="animate-spin mr-2" /> Đang tải...
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div>
        <h3 className="text-lg font-bold mb-1 flex items-center gap-2">
          <Wifi size={20} className="text-brand-primary" />
          MCP Gateway
        </h3>
        <p className="text-sm text-brand-muted">Cho phép AI agents (OpenClaw, n8n, Claude Code...) kết nối qua HTTP và quản lý {APP_NAME}</p>
      </div>

      <div className="flex items-center justify-between p-4 bg-slate-50 rounded-xl">
        <div className="flex items-center gap-3">
          <div className={`w-3 h-3 rounded-full ${status?.running ? "bg-emerald-500 animate-pulse" : "bg-red-400"}`} />
          <div>
            <p className="font-medium text-sm">{status?.running ? "Gateway đang chạy" : "Gateway không hoạt động"}</p>
            <p className="text-xs text-brand-muted">
              {status?.running
                ? `Port ${status.port} • ${status.has_api_keys ? "API key đã cấu hình ✓" : "Chưa có API key"}`
                : "Khởi động lại ứng dụng để bật Gateway"}
            </p>
          </div>
        </div>
        <button onClick={fetchStatus} className="p-2 hover:bg-slate-200 rounded-lg transition-colors cursor-pointer" title="Làm mới">
          <RefreshCw size={14} className="text-brand-muted" />
        </button>
      </div>

      <div className="p-5 bg-slate-50 rounded-2xl space-y-4">
        <h4 className="font-bold text-sm flex items-center gap-2">
          <Key size={16} className="text-amber-500" />
          API Key
        </h4>
        <p className="text-xs text-brand-muted">API key dùng để xác thực AI agent khi kết nối. Mỗi key chỉ hiển thị 1 lần sau khi tạo.</p>
        {!canManageGateway && (
          <p className="text-xs text-brand-muted">Chỉ admin mới có thể tạo API key mới.</p>
        )}

        <div className="flex items-center gap-2">
          <Input
            value={label}
            onChange={(event) => setLabel(event.target.value)}
            placeholder="Label (VD: zeroclaw, claude)"
            className="w-48"
            disabled={!canManageGateway}
          />
          <Button
            onClick={() => void handleGenerate()}
            disabled={!canManageGateway || generating}
            className="bg-brand-primary text-white rounded-xl"
          >
            {generating ? "Đang tạo..." : "Tạo API Key"}
          </Button>
        </div>

        {apiKey && (
          <div className="p-4 bg-white rounded-xl border border-slate-200 space-y-3">
            <div className="flex items-center justify-between">
              <span className="text-xs font-semibold text-amber-600">⚠️ Sao chép ngay — key sẽ không hiển thị lại!</span>
              <button onClick={() => setShowKey(!showKey)} className="text-brand-muted hover:text-brand-text cursor-pointer">
                {showKey ? <EyeOff size={14} /> : <Eye size={14} />}
              </button>
            </div>
            <div className="flex items-center gap-2">
              <code className="flex-1 text-xs bg-slate-100 px-3 py-2 rounded-lg font-mono break-all">
                {showKey ? apiKey : `${APP_API_KEY_PREFIX}${"•".repeat(32)}`}
              </code>
              <button
                onClick={() => {
                  void navigator.clipboard.writeText(apiKey);
                  toast.success("Đã copy API key!");
                }}
                className="p-2 hover:bg-slate-100 rounded-lg cursor-pointer"
                title="Copy"
              >
                <Copy size={14} />
              </button>
            </div>
          </div>
        )}
      </div>

      <div className="flex items-center justify-between p-4 bg-slate-50 rounded-xl">
        <div>
          <p className="font-medium text-sm">📋 Copy MCP Config</p>
          <p className="text-xs text-brand-muted">Dán vào OpenClaw hoặc MCP client hỗ trợ HTTP để kết nối</p>
        </div>
        <Button variant="outline" className="rounded-xl" onClick={copyMcpConfig}>
          <Copy size={14} className="mr-1.5" /> Copy Config
        </Button>
      </div>

      <div className="p-4 bg-blue-50/50 rounded-xl">
        <p className="text-sm font-medium text-blue-900 mb-2">📡 {MCP_TOOLS.length} MCP Tools available</p>
        <div className="grid grid-cols-2 gap-1 text-xs text-blue-700">
          {MCP_TOOLS.map((tool) => (
            <span key={tool.name} className={tool.writable ? "text-emerald-700 font-medium" : undefined}>
              • {tool.name}
              {tool.writable ? " ✏️" : ""}
            </span>
          ))}
        </div>
      </div>
    </div>
  );
}
