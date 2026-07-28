import { useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { formatAppError } from "@/lib/appError";
import { invokeCommand } from "@/lib/invokeCommand";
import type { DeclarationIdentity } from "@/types";

import {
  DOC_TYPES,
  DOC_TYPE_OTHER,
  GENDERS,
  NATIONALITY_HINTS,
  RESIDENCE_STATUSES,
  VIETNAM_ISO3,
  isForeign,
} from "./catalog";

interface ManualFormProps {
  /** Có giá trị = chế độ sửa: prefill và lưu theo id qua kbtt_update_identity. */
  initial?: DeclarationIdentity | null;
  onSaved?: (identityId: string, identity: DeclarationIdentity) => void;
  onCancel?: () => void;
}

interface FormState {
  full_name: string;
  dob: string;
  gender: string;
  nationality_iso3: string;
  doc_type_code: string;
  doc_type_name: string;
  doc_no: string;
  phone: string;
  residence_status: string;
  address_detail: string;
  passport_no: string;
  passport_expiry: string;
  visa_valid_until: string;
}

const EMPTY: FormState = {
  full_name: "",
  dob: "",
  gender: "M",
  nationality_iso3: VIETNAM_ISO3,
  doc_type_code: "1",
  doc_type_name: "",
  doc_no: "",
  phone: "",
  residence_status: "1",
  address_detail: "",
  passport_no: "",
  passport_expiry: "",
  visa_valid_until: "",
};

function orNull(value: string): string | null {
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function fromIdentity(identity: DeclarationIdentity): FormState {
  return {
    full_name: identity.full_name,
    dob: identity.dob,
    gender: identity.gender,
    nationality_iso3: identity.nationality_iso3,
    doc_type_code: identity.doc_type_code ?? "1",
    doc_type_name: identity.doc_type_name ?? "",
    doc_no: identity.doc_no ?? "",
    phone: identity.phone ?? "",
    residence_status: identity.residence_status ?? "1",
    address_detail: identity.address_detail ?? "",
    passport_no: identity.passport_no ?? "",
    passport_expiry: identity.passport_expiry ?? "",
    visa_valid_until: identity.visa_valid_until ?? "",
  };
}

/**
 * Form nhập tay (spec §6.4). Dùng khi ảnh không có QR/MRZ (CMND cũ, giấy khai
 * sinh, GPLX), khi decode fail, hoặc khi người vận hành chủ động chọn.
 *
 * Luôn lưu với `source = "manual"` / `confidence = "needs_review"` để `W05` nổ
 * và có người soi lại, và `doc_type_source = "human"` để `W06` KHÔNG nổ — người
 * tự chọn loại giấy tờ thì đó không còn là heuristic ngày cấp.
 */
export default function ManualForm({ initial, onSaved, onCancel }: ManualFormProps) {
  const [form, setForm] = useState<FormState>(
    initial ? fromIdentity(initial) : EMPTY,
  );
  const [saving, setSaving] = useState(false);

  const foreign = isForeign(form.nationality_iso3);
  const canSave =
    form.full_name.trim().length > 0 && form.dob.trim().length > 0 && !saving;

  const set = <K extends keyof FormState>(key: K, value: FormState[K]) =>
    setForm((prev) => ({ ...prev, [key]: value }));

  const handleSave = async () => {
    if (!canSave) return;
    setSaving(true);
    const identity: Omit<DeclarationIdentity, "id"> = {
      full_name: form.full_name.trim(),
      dob: form.dob,
      gender: form.gender,
      nationality_iso3: form.nationality_iso3.trim().toUpperCase(),
      doc_type_code: foreign ? null : form.doc_type_code,
      doc_type_source: "human",
      doc_type_name: foreign ? null : orNull(form.doc_type_name),
      doc_no: foreign ? null : orNull(form.doc_no),
      phone: orNull(form.phone),
      residence_status: foreign ? null : form.residence_status,
      address_detail: orNull(form.address_detail),
      passport_no: foreign ? orNull(form.passport_no) : null,
      passport_expiry: foreign ? orNull(form.passport_expiry) : null,
      visa_valid_until: foreign ? orNull(form.visa_valid_until) : null,
      name_confirmed_by_human: false,
      single_token_name_ok: false,
    };

    try {
      if (initial?.id) {
        await invokeCommand<void>("kbtt_update_identity", {
          identityId: initial.id,
          identity: { ...identity, id: initial.id },
          source: "manual",
          confidence: "needs_review",
        });
        toast.success("Đã sửa thông tin khách");
        onSaved?.(initial.id, { ...identity, id: initial.id });
      } else {
        const identityId = await invokeCommand<string>("kbtt_save_identity", {
          identity,
          source: "manual",
          confidence: "needs_review",
        });
        toast.success("Đã lưu danh tính");
        setForm(EMPTY);
        onSaved?.(identityId, { ...identity, id: identityId });
      }
    } catch (e) {
      toast.error(formatAppError(e));
    } finally {
      setSaving(false);
    }
  };

  const field = "w-full rounded-lg border border-slate-200 px-3 py-2 text-sm";
  const labelClass = "mb-1 block text-xs text-slate-600";

  return (
    <div className="rounded-2xl border border-slate-200 bg-white p-4">
      <h3 className="mb-3 text-sm font-bold">Nhập tay danh tính</h3>

      <div className="grid grid-cols-2 gap-3">
        <div className="col-span-2">
          <label htmlFor="kbtt-manual-name" className={labelClass}>
            Họ và tên
          </label>
          <input
            id="kbtt-manual-name"
            className={field}
            value={form.full_name}
            onChange={(e) => set("full_name", e.target.value)}
          />
        </div>

        <div>
          <label htmlFor="kbtt-manual-dob" className={labelClass}>
            Ngày sinh
          </label>
          <input
            id="kbtt-manual-dob"
            type="date"
            className={field}
            value={form.dob}
            onChange={(e) => set("dob", e.target.value)}
          />
        </div>

        <div>
          <label htmlFor="kbtt-manual-gender" className={labelClass}>
            Giới tính
          </label>
          <select
            id="kbtt-manual-gender"
            className={field}
            value={form.gender}
            onChange={(e) => set("gender", e.target.value)}
          >
            {GENDERS.map((g) => (
              <option key={g.code} value={g.code}>
                {g.code} - {g.label}
              </option>
            ))}
          </select>
        </div>

        <div>
          <label htmlFor="kbtt-manual-nat" className={labelClass}>
            Quốc tịch (ISO3)
          </label>
          <input
            id="kbtt-manual-nat"
            list="kbtt-nationality-hints"
            className={`${field} font-mono uppercase`}
            value={form.nationality_iso3}
            onChange={(e) =>
              set("nationality_iso3", e.target.value.toUpperCase())
            }
          />
          <datalist id="kbtt-nationality-hints">
            {NATIONALITY_HINTS.map((n) => (
              <option key={n.code} value={n.code}>
                {n.label}
              </option>
            ))}
          </datalist>
        </div>

        <div>
          <label htmlFor="kbtt-manual-phone" className={labelClass}>
            Điện thoại
          </label>
          <input
            id="kbtt-manual-phone"
            className={field}
            value={form.phone}
            onChange={(e) => set("phone", e.target.value)}
          />
        </div>

        {!foreign && (
          <>
            <div>
              <label htmlFor="kbtt-manual-doctype" className={labelClass}>
                Loại giấy tờ
              </label>
              <select
                id="kbtt-manual-doctype"
                className={field}
                value={form.doc_type_code}
                onChange={(e) => set("doc_type_code", e.target.value)}
              >
                {DOC_TYPES.map((d) => (
                  <option key={d.code} value={d.code}>
                    {d.code} - {d.label}
                  </option>
                ))}
              </select>
            </div>

            <div>
              <label htmlFor="kbtt-manual-docno" className={labelClass}>
                Số giấy tờ
              </label>
              <input
                id="kbtt-manual-docno"
                className={`${field} font-mono`}
                value={form.doc_no}
                onChange={(e) => set("doc_no", e.target.value)}
              />
            </div>

            {form.doc_type_code === DOC_TYPE_OTHER && (
              <div className="col-span-2">
                <label htmlFor="kbtt-manual-doctypename" className={labelClass}>
                  Tên giấy tờ (bắt buộc khi chọn Giấy Tờ Khác)
                </label>
                <input
                  id="kbtt-manual-doctypename"
                  className={field}
                  value={form.doc_type_name}
                  onChange={(e) => set("doc_type_name", e.target.value)}
                />
              </div>
            )}

            <div>
              <label htmlFor="kbtt-manual-residence" className={labelClass}>
                Nơi cư trú hiện nay
              </label>
              <select
                id="kbtt-manual-residence"
                className={field}
                value={form.residence_status}
                onChange={(e) => set("residence_status", e.target.value)}
              >
                {RESIDENCE_STATUSES.map((r) => (
                  <option key={r.code} value={r.code}>
                    {r.code} - {r.label}
                  </option>
                ))}
              </select>
            </div>
          </>
        )}

        {foreign && (
          <>
            <div>
              <label htmlFor="kbtt-manual-passport" className={labelClass}>
                Số hộ chiếu
              </label>
              <input
                id="kbtt-manual-passport"
                className={`${field} font-mono`}
                value={form.passport_no}
                onChange={(e) => set("passport_no", e.target.value)}
              />
            </div>

            <div>
              <label htmlFor="kbtt-manual-passportexp" className={labelClass}>
                Ngày hết hạn hộ chiếu
              </label>
              <input
                id="kbtt-manual-passportexp"
                type="date"
                className={field}
                value={form.passport_expiry}
                onChange={(e) => set("passport_expiry", e.target.value)}
              />
            </div>

            <div className="col-span-2">
              <label htmlFor="kbtt-manual-visa" className={labelClass}>
                Thời hạn tạm trú (bắt buộc, nhập tay)
              </label>
              <input
                id="kbtt-manual-visa"
                type="date"
                className={field}
                value={form.visa_valid_until}
                onChange={(e) => set("visa_valid_until", e.target.value)}
              />
              <p className="mt-1 text-xs text-slate-500">
                Không phải ngày hết hạn hộ chiếu. Lấy từ dấu nhập cảnh hoặc thị
                thực.
              </p>
            </div>
          </>
        )}

        <div className="col-span-2">
          <label htmlFor="kbtt-manual-address" className={labelClass}>
            Địa chỉ chi tiết
          </label>
          <input
            id="kbtt-manual-address"
            className={field}
            value={form.address_detail}
            onChange={(e) => set("address_detail", e.target.value)}
          />
        </div>
      </div>

      <div className="mt-4 flex items-center gap-2">
        <Button
          type="button"
          className="rounded-xl"
          disabled={!canSave}
          onClick={handleSave}
        >
          {saving ? "Đang lưu..." : "Lưu danh tính"}
        </Button>
        {onCancel && (
          <Button
            type="button"
            variant="ghost"
            className="rounded-xl"
            onClick={onCancel}
          >
            Hủy
          </Button>
        )}
      </div>
    </div>
  );
}
