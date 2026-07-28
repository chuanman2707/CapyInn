import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import Modal from "@/components/ui/Modal";
import { Button } from "@/components/ui/button";
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
import { declarationErrorMessage } from "./declarationError";
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
  const [noteDraft, setNoteDraft] = useState(row.stay_reason_note ?? "");
  // FINDING E: "Xóa" xóa VĨNH VIỄN — batch entries, liên kết, và cả dòng
  // danh tính — không như "Gác lại" (chỉ tạm giấu, đưa lại được). Hai nút
  // này đứng cách nhau vài pixel trong cùng một dòng chữ xám nhỏ; một cú
  // bấm nhầm không được phép xóa thẳng, phải hỏi lại và nêu đích danh khách.
  const [confirmingDiscard, setConfirmingDiscard] = useState(false);
  // FINDING C1: E16 ("chưa chọn phòng") trỏ vào chính ô Phòng trên thẻ này,
  // không mở ManualForm — form đó không có ô ngày đến/ngày đi nào để sửa,
  // vì hai ngày đó đến từ lượt lưu trú chứ không phải danh tính.
  const roomSelectRef = useRef<HTMLSelectElement>(null);

  // Đồng bộ lại khi đổi khách (link khác) hoặc khi note được nạp lại từ
  // server (ví dụ sau khi một thẻ khác kích hoạt reload) — KHÔNG chạy khi
  // người dùng đang gõ dở, vì lúc đó `row.stay_reason_note` chưa đổi.
  useEffect(() => {
    setNoteDraft(row.stay_reason_note ?? "");
  }, [row.link_id, row.stay_reason_note]);

  const blocking = findings.some((f) => f.severity === "blocking");
  const border = blocking
    ? "border-red-300"
    : findings.length > 0
      ? "border-amber-300"
      : "border-slate-200";

  // Phòng hiện tại của link: đọc thẳng `row.stay_id`, KHÔNG đoán qua
  // room_no. `kbtt_list_stays` (load_stays_for_declaration) chỉ trả các
  // booking status = 'active' — nếu khách đã trả phòng/hủy trước khi khai
  // xong, `row.stay_id` vẫn là booking thật đó nhưng nó không còn trong
  // `stays`. Đoán qua room_no từng khiến chỗ này rơi về "chưa xác định
  // phòng" trong đúng lúc đó, và đổi lý do lưu trú (không đụng ô phòng) sẽ
  // gửi `stayId: null` xóa mất liên kết phòng có thật (FINDING 1).
  const currentStay = row.stay_id ?? STAY_NONE;

  // `row.stay_id` có giá trị nhưng không nằm trong `stays` đang active: booking
  // đã bị trả phòng/hủy. Không có option nào khớp thì `<select>` hiện trống —
  // người vận hành tưởng chưa chọn gì và dễ bấm sang ô khác làm mất giá trị.
  // Thêm hẳn một option (khóa lại, không cho chọn) để trạng thái hiện đúng và
  // được giữ nguyên cho tới khi người vận hành chủ động chọn phòng khác.
  const staleStayId =
    row.stay_id != null && !stays.some((s) => s.stay_id === row.stay_id) ? row.stay_id : null;

  // Di sản spec gốc §7: app chỉ XẾP THỨ TỰ theo độ giống tên, không tự chọn.
  const rankedStays = [...stays].sort(
    (a, b) =>
      nameScore(row.full_name, b.guest_name ?? "") -
      nameScore(row.full_name, a.guest_name ?? ""),
  );

  // FINDING 1: một số lỗi từ `kbtt_update_link` (VD: "lượt lưu trú vừa chọn
  // đã kết thúc") chỉ đúng vì danh sách phòng phía client đang cũ — cách
  // duy nhất để người vận hành thật sự sửa được là danh sách đó được tải
  // lại. Trước đây `onChanged()` chỉ chạy khi `fn()` thành công, nên gặp
  // đúng lỗi này thẻ vẫn hiện y nguyên danh sách phòng cũ, người vận hành
  // chọn lại đúng cái phòng vẫn đang hiện trong danh sách đó và nhận lại
  // đúng lỗi — màn hình không có nút "tải lại" nào khác để bấm.
  //
  // Chọn tải lại vô điều kiện ở MỌI lỗi của `call()` (đưa `onChanged()` vào
  // `finally`), không chỉ khớp riêng câu lỗi này bằng string-match. Rust trả
  // lỗi dạng chuỗi trần (không có mã lỗi), nên match theo nội dung câu sẽ vỡ
  // âm thầm nếu câu đổi chữ sau này — không có gì báo cho biết nhánh "tải
  // lại khi lỗi" đã ngừng khớp. Xét từng lệnh `call()` đang bọc (đổi
  // phòng/lý do/ghi chú, Gác lại, Đưa lại, Xóa): tất cả các lỗi còn lại của
  // chúng — "đã nằm trong lô đã đối soát", "không tìm thấy khai báo cần
  // sửa", lỗi đọc DB — đều là những ca mà tải lại đưa màn hình về đúng
  // trạng thái server (thẻ biến mất nếu link không còn, hoặc y nguyên nếu
  // vẫn còn) chứ không có ca nào tải lại làm mất thêm gì. Cái giá là một
  // lượt gọi `kbtt_pending_rows`/`kbtt_list_stays` thừa mỗi lần lỗi — rẻ hơn
  // nhiều so với một thẻ kẹt cứng.
  const call = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await fn();
    } catch (e) {
      toast.error(declarationErrorMessage(e));
    } finally {
      onChanged();
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
            ref={roomSelectRef}
            className="w-full rounded-lg border border-slate-200 bg-white px-3 py-2 text-sm"
            value={currentStay}
            disabled={busy}
            onChange={(e) => void updateLink(e.target.value, row.stay_reason, row.stay_reason_note)}
          >
            <option value={STAY_NONE}>Chưa xác định phòng</option>
            {staleStayId && (
              <option value={staleStayId} disabled>
                Phòng cũ (khách đã trả phòng)
              </option>
            )}
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

        {row.stay_reason === STAY_REASON_OTHER && (
          <div className="col-span-2">
            <label
              htmlFor={`reason-note-${row.link_id}`}
              className="mb-1 block text-xs text-slate-600"
            >
              Lý do cụ thể
            </label>
            <input
              id={`reason-note-${row.link_id}`}
              className="w-full rounded-lg border border-slate-200 bg-white px-3 py-2 text-sm"
              value={noteDraft}
              disabled={busy}
              onChange={(e) => setNoteDraft(e.target.value)}
              onBlur={() => {
                const trimmed = noteDraft.trim();
                if (trimmed !== (row.stay_reason_note ?? "")) {
                  void updateLink(currentStay, row.stay_reason, trimmed === "" ? null : trimmed);
                }
              }}
            />
          </div>
        )}
      </div>

      {findings.length > 0 && (
        <ul className="mt-3 space-y-1">
          {findings.map((f) => {
            // FINDING C1: E16 ("chưa chọn phòng") không có gì để sửa trong
            // ManualForm — cái nó cần đã nằm ngay trên thẻ này, ô Phòng.
            const isRoomMissing = f.code === "E16";
            return (
              <li key={`${f.code}-${f.field ?? ""}`}>
                <button
                  type="button"
                  onClick={() =>
                    isRoomMissing ? roomSelectRef.current?.focus() : setEditing(true)
                  }
                  className={`text-left text-sm underline-offset-2 hover:underline ${
                    f.severity === "blocking" ? "text-red-700" : "text-amber-700"
                  }`}
                >
                  {f.severity === "blocking" ? "⛔" : "⚠"} {findingLine(f)}{" "}
                  <span className="font-mono text-[10px] text-slate-400">{f.code}</span>
                </button>
              </li>
            );
          })}
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
            aria-label={`Đưa lại ${row.full_name}`}
            className="text-slate-500 underline hover:text-slate-800"
            onClick={() => void call(() => invokeCommand<void>("kbtt_release", { linkId: row.link_id }))}
          >
            Đưa lại
          </button>
        ) : (
          <button
            type="button"
            disabled={busy}
            aria-label={`Gác lại ${row.full_name}`}
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
          onClick={() => setConfirmingDiscard(true)}
        >
          Xóa
        </button>
      </div>

      {confirmingDiscard && (
        <Modal title="Xóa khách này">
          <div className="space-y-4">
            <p className="text-sm text-slate-600">
              Xóa <strong>{row.full_name}</strong> khỏi hệ thống — xóa cả liên
              kết lưu trú lẫn dòng danh tính, không khôi phục lại được. Khác
              với &quot;Gác lại&quot; (chỉ tạm giấu, đưa lại được), việc này
              là vĩnh viễn.
            </p>
            <div className="flex justify-end gap-2">
              <Button
                type="button"
                variant="outline"
                onClick={() => setConfirmingDiscard(false)}
              >
                Hủy
              </Button>
              <Button
                type="button"
                variant="destructive"
                aria-label={`Xác nhận xóa vĩnh viễn ${row.full_name}`}
                onClick={() => {
                  setConfirmingDiscard(false);
                  void call(() =>
                    invokeCommand<void>("kbtt_discard", { linkId: row.link_id }),
                  );
                }}
              >
                Xóa vĩnh viễn
              </Button>
            </div>
          </div>
        </Modal>
      )}
    </div>
  );
}
