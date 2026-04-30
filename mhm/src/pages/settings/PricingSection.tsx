import { useEffect, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { DollarSign } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { assertNonNegativeMoneyVnd } from "@/lib/money";
import type { PricingRuleData } from "@/types";

import DynamicRoomTypeSelect from "./DynamicRoomTypeSelect";

export default function PricingSection() {
  const [rules, setRules] = useState<PricingRuleData[]>([]);
  const [editing, setEditing] = useState<string | null>(null);
  const [form, setForm] = useState<PricingRuleData>({
    room_type: "",
    hourly_rate: 80000,
    overnight_rate: 300000,
    daily_rate: 400000,
    early_checkin_surcharge_pct: 30,
    late_checkout_surcharge_pct: 30,
    weekend_uplift_pct: 20,
  });

  useEffect(() => {
    invoke<PricingRuleData[]>("get_pricing_rules").then(setRules).catch(() => {});
  }, []);

  const startEdit = (rule: PricingRuleData) => {
    setEditing(rule.room_type);
    setForm({ ...rule });
  };

  const handleSave = async () => {
    if (!form.room_type) {
      toast.error("Chọn loại phòng trước khi lưu");
      return;
    }

    try {
      const hourlyRate = assertNonNegativeMoneyVnd(form.hourly_rate, "hourly_rate");
      const overnightRate = assertNonNegativeMoneyVnd(form.overnight_rate, "overnight_rate");
      const dailyRate = assertNonNegativeMoneyVnd(form.daily_rate, "daily_rate");
      await invoke("save_pricing_rule", {
        roomType: form.room_type,
        hourlyRate,
        overnightRate,
        dailyRate,
        earlyPct: form.early_checkin_surcharge_pct,
        latePct: form.late_checkout_surcharge_pct,
        weekendPct: form.weekend_uplift_pct,
      });
      toast.success("Đã lưu bảng giá!");
      setEditing(null);
      invoke<PricingRuleData[]>("get_pricing_rules").then(setRules);
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error) || "Lỗi lưu bảng giá");
    }
  };

  const fmtK = (value: number) => `${(value / 1000).toFixed(0)}k`;

  return (
    <div className="space-y-6">
      <div>
        <h3 className="text-lg font-bold mb-1 flex items-center gap-2">
          <DollarSign size={20} className="text-emerald-500" />
          Bảng giá theo loại phòng
        </h3>
        <p className="text-sm text-brand-muted">Cấu hình giá theo giờ, qua đêm, theo ngày cho từng loại phòng</p>
      </div>

      {rules.length > 0 && (
        <div className="space-y-2">
          {rules.map((rule) => (
            <div key={rule.room_type} className="flex items-center justify-between p-4 bg-slate-50 rounded-xl">
              <div>
                <p className="font-semibold text-sm capitalize">{rule.room_type}</p>
                <p className="text-xs text-brand-muted">
                  ⏱ {fmtK(rule.hourly_rate)} / 🌙 {fmtK(rule.overnight_rate)} / 📅 {fmtK(rule.daily_rate)} &nbsp;|&nbsp; Sớm +
                  {rule.early_checkin_surcharge_pct}% &nbsp; Trễ +{rule.late_checkout_surcharge_pct}% &nbsp; T7-CN +
                  {rule.weekend_uplift_pct}%
                </p>
              </div>
              <Button variant="outline" size="sm" className="rounded-lg" onClick={() => startEdit(rule)}>
                Sửa
              </Button>
            </div>
          ))}
        </div>
      )}

      <div className="p-5 bg-slate-50 rounded-2xl space-y-4">
        <h4 className="font-bold text-sm">{editing ? `Sửa: ${editing}` : "Thêm bảng giá"}</h4>
        <div className="grid grid-cols-2 gap-3">
          <div>
            <Label>Loại phòng</Label>
            <DynamicRoomTypeSelect value={form.room_type} onChange={(roomType) => setForm({ ...form, room_type: roomType })} disabled={Boolean(editing)} />
          </div>
          <div />
          <Field label="⏱ Giá theo giờ">
            <Input type="number" value={form.hourly_rate} onChange={(event) => setForm({ ...form, hourly_rate: Number(event.target.value) })} className="mt-1.5" />
          </Field>
          <Field label="🌙 Giá qua đêm">
            <Input type="number" value={form.overnight_rate} onChange={(event) => setForm({ ...form, overnight_rate: Number(event.target.value) })} className="mt-1.5" />
          </Field>
          <Field label="📅 Giá theo ngày">
            <Input type="number" value={form.daily_rate} onChange={(event) => setForm({ ...form, daily_rate: Number(event.target.value) })} className="mt-1.5" />
          </Field>
          <div />
          <Field label="% phụ thu check-in sớm">
            <Input
              type="number"
              value={form.early_checkin_surcharge_pct}
              onChange={(event) => setForm({ ...form, early_checkin_surcharge_pct: Number(event.target.value) })}
              className="mt-1.5 w-24"
            />
          </Field>
          <Field label="% phụ thu check-out trễ">
            <Input
              type="number"
              value={form.late_checkout_surcharge_pct}
              onChange={(event) => setForm({ ...form, late_checkout_surcharge_pct: Number(event.target.value) })}
              className="mt-1.5 w-24"
            />
          </Field>
          <Field label="% phụ thu cuối tuần">
            <Input
              type="number"
              value={form.weekend_uplift_pct}
              onChange={(event) => setForm({ ...form, weekend_uplift_pct: Number(event.target.value) })}
              className="mt-1.5 w-24"
            />
          </Field>
        </div>
        <div className="flex gap-2">
          <Button onClick={() => void handleSave()} disabled={!form.room_type} className="bg-brand-primary text-white rounded-xl">
            {editing ? "Cập nhật" : "Thêm"}
          </Button>
          {editing && (
            <Button variant="outline" className="rounded-xl" onClick={() => setEditing(null)}>
              Hủy
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div>
      <Label>{label}</Label>
      {children}
    </div>
  );
}
