import { useState, useEffect } from "react";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { createCorrelationId } from "@/lib/correlationId";
import { formatAppError } from "@/lib/appError";
import { invokeCommand } from "@/lib/invokeCommand";
import { useAuthStore } from "@/stores/useAuthStore";
import { Moon, TrendingUp, Home, DollarSign, AlertCircle } from "lucide-react";
import { fmtMoney } from "@/lib/format";
import type { AuditLog } from "@/types";

export default function NightAudit() {
    const { isAdmin } = useAuthStore();
    const [logs, setLogs] = useState<AuditLog[]>([]);
    const [auditDate, setAuditDate] = useState(() => {
        const d = new Date();
        return d.toISOString().slice(0, 10);
    });
    const [notes, setNotes] = useState("");
    const [running, setRunning] = useState(false);

    useEffect(() => {
        invoke<AuditLog[]>("get_audit_logs").then(setLogs).catch(() => { });
    }, []);

    const handleRunAudit = async () => {
        if (!isAdmin()) {
            toast.error("Chỉ Admin mới có thể chạy Night Audit");
            return;
        }
        setRunning(true);
        try {
            const correlationId = createCorrelationId();
            const result = await invokeCommand<AuditLog>(
                "run_night_audit",
                {
                    auditDate,
                    notes: notes || null,
                },
                {
                    correlationId,
                    monitoringContext: {
                        notes_present: Boolean(notes.trim()),
                    },
                },
            );
            toast.success(`Night Audit ngày ${auditDate} hoàn tất!`);
            setLogs((prev) => [result, ...prev]);
            setNotes("");
        } catch (e: any) {
            toast.error(formatAppError(e));
        } finally {
            setRunning(false);
        }
    };

    return (
        <div className="space-y-6">
            {/* Run Audit Card */}
            {isAdmin() && (
                <Card className="p-6 bg-gradient-to-br from-indigo-500 to-purple-600 text-white border-0">
                    <div className="flex items-center gap-3 mb-4">
                        <Moon size={24} />
                        <h2 className="text-xl font-bold">Night Audit</h2>
                    </div>
                    <p className="text-white/80 text-sm mb-4">
                        Chạy báo cáo cuối ngày — tính tổng doanh thu, chi phí, công suất phòng. Dữ liệu sẽ bị khóa sau khi audit.
                    </p>
                    <div className="flex items-end gap-3">
                        <div>
                            <label className="text-xs text-white/70 block mb-1">Ngày audit</label>
                            <Input
                                type="date"
                                value={auditDate}
                                onChange={(e) => setAuditDate(e.target.value)}
                                className="bg-white/20 border-white/30 text-white placeholder:text-white/50 w-44"
                            />
                        </div>
                        <div className="flex-1">
                            <label className="text-xs text-white/70 block mb-1">Ghi chú</label>
                            <Input
                                value={notes}
                                onChange={(e) => setNotes(e.target.value)}
                                placeholder="VD: Đã kiểm tra kho..."
                                className="bg-white/20 border-white/30 text-white placeholder:text-white/50"
                            />
                        </div>
                        <Button
                            onClick={handleRunAudit}
                            disabled={running}
                            className="bg-white text-indigo-600 hover:bg-white/90 font-bold rounded-xl px-6"
                        >
                            {running ? "Đang chạy..." : "🌙 Chạy Audit"}
                        </Button>
                    </div>
                </Card>
            )}

            {/* Audit History */}
            <div>
                <h3 className="text-lg font-bold mb-3">Lịch sử Audit</h3>
                {logs.length === 0 ? (
                    <Card className="p-8 text-center">
                        <AlertCircle size={32} className="mx-auto text-brand-muted mb-2" />
                        <p className="text-sm text-brand-muted">Chưa có bản audit nào</p>
                    </Card>
                ) : (
                    <div className="space-y-3">
                        {logs.map((log) => (
                            <Card key={log.id} className="p-5">
                                <div className="flex items-center justify-between mb-3">
                                    <div className="flex items-center gap-3">
                                        <div className="w-10 h-10 rounded-xl bg-indigo-100 text-indigo-600 flex items-center justify-center">
                                            <Moon size={18} />
                                        </div>
                                        <div>
                                            <p className="font-bold text-sm">{log.audit_date}</p>
                                            {log.notes && <p className="text-xs text-brand-muted">{log.notes}</p>}
                                        </div>
                                    </div>
                                    <div className="text-right">
                                        <p className="text-xl font-bold text-emerald-600">{fmtMoney(log.total_revenue)}</p>
                                        <p className="text-xs text-brand-muted">Tổng doanh thu</p>
                                    </div>
                                </div>

                                <div className="grid grid-cols-4 gap-3">
                                    <div className="p-3 bg-slate-50 rounded-xl text-center">
                                        <Home size={14} className="mx-auto text-blue-500 mb-1" />
                                        <p className="text-sm font-bold">{fmtMoney(log.room_revenue)}</p>
                                        <p className="text-[10px] text-brand-muted">Doanh thu phòng</p>
                                    </div>
                                    <div className="p-3 bg-slate-50 rounded-xl text-center">
                                        <DollarSign size={14} className="mx-auto text-emerald-500 mb-1" />
                                        <p className="text-sm font-bold">{fmtMoney(log.folio_revenue)}</p>
                                        <p className="text-[10px] text-brand-muted">Dịch vụ thêm</p>
                                    </div>
                                    <div className="p-3 bg-slate-50 rounded-xl text-center">
                                        <TrendingUp size={14} className="mx-auto text-amber-500 mb-1" />
                                        <p className="text-sm font-bold">{log.occupancy_pct}%</p>
                                        <p className="text-[10px] text-brand-muted">Công suất ({log.rooms_sold}/{log.total_rooms})</p>
                                    </div>
                                    <div className="p-3 bg-red-50 rounded-xl text-center">
                                        <p className="text-sm font-bold text-red-600">-{fmtMoney(log.total_expenses)}</p>
                                        <p className="text-[10px] text-brand-muted">Chi phí</p>
                                    </div>
                                </div>
                            </Card>
                        ))}
                    </div>
                )}
            </div>
        </div>
    );
}
