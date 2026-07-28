import { useState } from "react";
import { toast } from "sonner";

import { formatAppError } from "@/lib/appError";
import { invokeCommand } from "@/lib/invokeCommand";
import type {
  DeclarationFinding,
  DeclarationIdentity,
  DeclarationRow,
  StayInfo,
} from "@/types";

import {
  STAY_REASONS,
  STAY_REASON_OTHER,
  findingText,
  isForeign,
} from "./catalog";
import ManualForm from "./ManualForm";
import { nameScore } from "./nameMatch";

/** Giá trị chọn "chưa xác định phòng" — giữ nguyên hằng của màn cũ. */
const STAY_NONE = "__chua_co_phong__";

interface GuestCardProps {
  row: DeclarationRow;
  stays: StayInfo[];
  findings: DeclarationFinding[];
  onChanged: () => void;
}

/**
 * Câu hiện cho từng finding. Mọi mã đi qua `findingText` (bản dịch tiếng
 * người chung), TRỪ `E01`: bản dịch chung của nó ("Thiếu thông tin bắt buộc")
 * không nói field nào thiếu, trong khi message gốc của validator liệt kê
 * đích danh ("Thiếu field bắt buộc: họ tên, ngày sinh"). Giữ câu chung làm
 * câu dẫn (đồng giọng với các finding khác, có "bấm để bổ sung") rồi nối
 * thêm danh sách field cụ thể từ message gốc — người vận hành vừa biết phải
 * bấm, vừa biết bấm xong cần điền gì mà không cần mở form ra mới thấy.
 */
function findingLine(f: DeclarationFinding): string {
  if (f.code === "E01") {
    return `${findingText(f)} ${f.message}`;
  }
  return findingText(f);
}

/**
 * Một thẻ = một khách (spec UX §4.1). Phòng và lý do sửa tại chỗ; lỗi của
 * validator hiện thành câu tiếng người và bấm được để mở form sửa.
 */
export default function GuestCard({ row, stays, findings, onChanged }: GuestCardProps) {
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState(false);

  const blocking = findings.some((f) => f.severity === "blocking");
  const border = blocking
    ? "border-red-300"
    : findings.length > 0
      ? "border-amber-300"
      : "border-slate-200";

  // Phòng hiện tại của link: so theo room_no. Sound vì `kbtt_list_stays`
  // (load_stays_for_declaration) chỉ trả các booking status = 'active', và
  // hệ thống booking chỉ cho một booking active trên mỗi room_id tại một thời
  // điểm (xem các truy vấn "WHERE room_id = ? AND status = 'active' LIMIT 1"
  // ở room_queries.rs) — nên room_no không lặp trong danh sách `stays` này,
  // so theo room_no ở đây tương đương so theo stay_id. Khi row.room_no là
  // null, không có stay nào khớp (rooms luôn có tên nên room_no trong `stays`
  // không bao giờ null) và select rơi về STAY_NONE — đúng ý.
  const currentStay = stays.find((s) => s.room_no === row.room_no)?.stay_id ?? STAY_NONE;

  // Di sản spec gốc §7: app chỉ XẾP THỨ TỰ theo độ giống tên, không tự chọn.
  const rankedStays = [...stays].sort(
    (a, b) =>
      nameScore(row.full_name, b.guest_name ?? "") -
      nameScore(row.full_name, a.guest_name ?? ""),
  );

  const call = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await fn();
      onChanged();
    } catch (e) {
      toast.error(formatAppError(e));
    } finally {
      setBusy(false);
    }
  };

  const updateLink = (stayId: string, stayReason: string, note: string | null) =>
    call(() =>
      invokeCommand<void>("kbtt_update_link", {
        linkId: row.link_id,
        stayId: stayId === STAY_NONE ? null : stayId,
        stayReason,
        note,
      }),
    );

  const identityForEdit: DeclarationIdentity = {
    id: row.identity_id,
    full_name: row.full_name,
    dob: row.dob,
    gender: row.gender,
    nationality_iso3: row.nationality_iso3,
    doc_type_code: row.doc_type_code,
    doc_type_name: row.doc_type_name,
    doc_no: row.doc_no,
    phone: row.phone,
    residence_status: row.residence_status,
    address_detail: row.address_detail,
    passport_no: row.passport_no,
    passport_expiry: row.passport_expiry,
    visa_valid_until: row.visa_valid_until,
    name_confirmed_by_human: row.name_confirmed_by_human,
    single_token_name_ok: row.single_token_name_ok,
  };

  return (
    <div className={`rounded-xl border bg-white p-4 ${border}`}>
      <div className="flex flex-wrap items-baseline gap-2">
        {isForeign(row.nationality_iso3) && <span aria-label="Khách nước ngoài">🌐</span>}
        <span className="font-semibold">{row.full_name}</span>
        <span className="text-sm text-brand-muted">
          {row.dob}
          {" · "}
          {row.doc_no ?? row.passport_no ?? "chưa có số giấy tờ"}
        </span>
      </div>

      <div className="mt-3 grid grid-cols-2 gap-3">
        <div>
          <label htmlFor={`room-${row.link_id}`} className="mb-1 block text-xs text-slate-600">
            Phòng
          </label>
          <select
            id={`room-${row.link_id}`}
            className="w-full rounded-lg border border-slate-200 bg-white px-3 py-2 text-sm"
            value={currentStay}
            disabled={busy}
            onChange={(e) => void updateLink(e.target.value, row.stay_reason, row.stay_reason_note)}
          >
            <option value={STAY_NONE}>Chưa xác định phòng</option>
            {rankedStays.map((s) => (
              <option key={s.stay_id} value={s.stay_id}>
                {s.room_no ? `Phòng ${s.room_no}` : "Chưa có phòng"} · {s.guest_name ?? "—"} ·{" "}
                {s.check_in} → {s.expected_out}
              </option>
            ))}
          </select>
        </div>
        <div>
          <label htmlFor={`reason-${row.link_id}`} className="mb-1 block text-xs text-slate-600">
            Lý do lưu trú
          </label>
          <select
            id={`reason-${row.link_id}`}
            className="w-full rounded-lg border border-slate-200 bg-white px-3 py-2 text-sm"
            value={row.stay_reason}
            disabled={busy}
            onChange={(e) =>
              void updateLink(
                currentStay,
                e.target.value,
                e.target.value === STAY_REASON_OTHER ? row.stay_reason_note : null,
              )
            }
          >
            {STAY_REASONS.map((r) => (
              <option key={r.code} value={r.code}>
                {r.label}
              </option>
            ))}
          </select>
        </div>
      </div>

      {findings.length > 0 && (
        <ul className="mt-3 space-y-1">
          {findings.map((f) => (
            <li key={`${f.code}-${f.field ?? ""}`}>
              <button
                type="button"
                onClick={() => setEditing(true)}
                className={`text-left text-sm underline-offset-2 hover:underline ${
                  f.severity === "blocking" ? "text-red-700" : "text-amber-700"
                }`}
              >
                {f.severity === "blocking" ? "⛔" : "⚠"} {findingLine(f)}{" "}
                <span className="font-mono text-[10px] text-slate-400">{f.code}</span>
              </button>
            </li>
          ))}
        </ul>
      )}

      {editing && (
        <div className="mt-3">
          <ManualForm
            initial={identityForEdit}
            onCancel={() => setEditing(false)}
            onSaved={() => {
              setEditing(false);
              onChanged();
            }}
          />
        </div>
      )}

      <div className="mt-3 flex justify-end gap-2 text-xs">
        {row.held ? (
          <button
            type="button"
            disabled={busy}
            className="text-slate-500 underline hover:text-slate-800"
            onClick={() => void call(() => invokeCommand<void>("kbtt_release", { linkId: row.link_id }))}
          >
            Đưa lại
          </button>
        ) : (
          <button
            type="button"
            disabled={busy}
            className="text-slate-500 underline hover:text-slate-800"
            onClick={() => void call(() => invokeCommand<void>("kbtt_hold", { linkId: row.link_id }))}
          >
            Gác lại
          </button>
        )}
        <button
          type="button"
          disabled={busy}
          aria-label={`Xóa ${row.full_name}`}
          className="text-slate-400 underline hover:text-red-600"
          onClick={() => void call(() => invokeCommand<void>("kbtt_discard", { linkId: row.link_id }))}
        >
          Xóa
        </button>
      </div>
    </div>
  );
}
