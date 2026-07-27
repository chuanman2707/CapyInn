import { useCallback, useEffect, useMemo, useState } from "react";
import { AlertTriangle } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
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
  STAY_REASON_DEFAULT,
  STAY_REASON_OTHER,
  isForeign,
} from "./catalog";
import { nameScore } from "./nameMatch";

interface PendingListProps {
  /** Danh tính vừa lưu — chỉ dùng để chọn sẵn, dữ liệu thật đọc từ DB. */
  identity?: DeclarationIdentity | null;
  onLinked?: () => void;
  onSelectionChange?: (linkIds: string[]) => void;
  onRowsChange?: (rows: DeclarationRow[]) => void;
  reloadKey?: number;
}

/**
 * Giá trị chọn "khai báo trước, chưa biết phòng".
 *
 * Cố ý KHÔNG dùng chuỗi rỗng: rỗng là "chưa chọn gì". Người vận hành phải chủ
 * động chọn mục này, không ai lỡ tay khai báo thiếu phòng vì quên chọn.
 */
const STAY_NONE = "__chua_co_phong__";

function rankStays(stays: StayInfo[], fullName: string): StayInfo[] {
  return [...stays].sort(
    (a, b) =>
      nameScore(fullName, b.guest_name ?? "") -
      nameScore(fullName, a.guest_name ?? ""),
  );
}

export default function PendingList({
  identity,
  onLinked,
  onSelectionChange,
  onRowsChange,
  reloadKey = 0,
}: PendingListProps) {
  const [stays, setStays] = useState<StayInfo[]>([]);
  const [rows, setRows] = useState<DeclarationRow[]>([]);
  const [findings, setFindings] = useState<DeclarationFinding[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const [stayId, setStayId] = useState("");
  const [stayReason, setStayReason] = useState(STAY_REASON_DEFAULT);
  const [stayReasonNote, setStayReasonNote] = useState("");
  const [linking, setLinking] = useState(false);
  /** Danh tính đã lưu nhưng chưa ghép — đọc từ DB, không giữ trong state. */
  const [waiting, setWaiting] = useState<DeclarationIdentity[]>([]);
  const [activeId, setActiveId] = useState("");

  useEffect(() => {
    invokeCommand<StayInfo[]>("kbtt_list_stays")
      .then(setStays)
      .catch(() => setStays([]));
  }, [reloadKey]);

  const loadWaiting = useCallback(() => {
    invokeCommand<DeclarationIdentity[]>("kbtt_unlinked_identities")
      .then((data) => setWaiting(data ?? []))
      .catch(() => setWaiting([]));
  }, []);

  useEffect(() => {
    loadWaiting();
  }, [loadWaiting, reloadKey]);

  // Ảnh vừa thả xong thì chọn sẵn đúng người đó.
  useEffect(() => {
    if (identity?.id) setActiveId(identity.id);
  }, [identity]);

  const loadRows = useCallback(() => {
    invokeCommand<DeclarationRow[]>("kbtt_pending_rows")
      .then((data) => setRows(data ?? []))
      .catch(() => setRows([]));
  }, []);

  useEffect(() => {
    loadRows();
  }, [loadRows, reloadKey]);

  useEffect(() => {
    onRowsChange?.(rows);
    const linkIds = rows.map((r) => r.link_id);
    setSelected(linkIds);
    if (linkIds.length === 0) {
      setFindings([]);
      return;
    }
    let cancelled = false;
    invokeCommand<DeclarationFinding[]>("kbtt_validate", { linkIds })
      .then((data) => {
        if (!cancelled) setFindings(data ?? []);
      })
      .catch(() => {
        if (!cancelled) setFindings([]);
      });
    return () => {
      cancelled = true;
    };
    // onRowsChange intentionally omitted: parent callbacks are not stable.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rows]);

  useEffect(() => {
    onSelectionChange?.(selected);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected]);

  /**
   * Người đang chờ ghép. Nguồn là DB, nên đổi sang tab khác rồi quay lại vẫn
   * còn — trước đây nó nằm trong `useState` và bị hủy cùng component.
   */
  const activeIdentity =
    waiting.find((w) => w.id === activeId) ?? waiting[0] ?? null;

  const rankedStays = useMemo(
    () => rankStays(stays, activeIdentity?.full_name ?? ""),
    [stays, activeIdentity],
  );

  const findingsByLink = useMemo(() => {
    const map = new Map<string, DeclarationFinding[]>();
    for (const f of findings) {
      const list = map.get(f.link_id) ?? [];
      list.push(f);
      map.set(f.link_id, list);
    }
    return map;
  }, [findings]);

  const foreignRows = rows.filter((r) => isForeign(r.nationality_iso3));
  const vnRows = rows.filter((r) => !isForeign(r.nationality_iso3));

  const canLink =
    stayId !== "" &&
    !linking &&
    (stayReason !== STAY_REASON_OTHER || stayReasonNote.trim().length > 0);

  const handleLink = async () => {
    if (!activeIdentity || !canLink) return;
    setLinking(true);
    try {
      await invokeCommand<string>("kbtt_link", {
        identityId: activeIdentity.id,
        stayId: stayId === STAY_NONE ? null : stayId,
        stayReason,
        note: stayReasonNote.trim() === "" ? null : stayReasonNote.trim(),
      });
      toast.success(
        stayId === STAY_NONE
          ? "Đã khai báo, chưa gắn phòng"
          : "Đã ghép khách vào lượt lưu trú",
      );
      setStayId("");
      setStayReason(STAY_REASON_DEFAULT);
      setStayReasonNote("");
      loadRows();
      loadWaiting();
      onLinked?.();
    } catch (e) {
      toast.error(formatAppError(e));
    } finally {
      setLinking(false);
    }
  };

  const handleDiscard = async (id: string, name: string) => {
    try {
      await invokeCommand<void>("kbtt_discard_identity", { identityId: id });
      toast.success(`Đã bỏ ${name}`);
      loadWaiting();
    } catch (e) {
      toast.error(formatAppError(e));
    }
  };

  const toggle = (linkId: string) =>
    setSelected((prev) =>
      prev.includes(linkId)
        ? prev.filter((id) => id !== linkId)
        : [...prev, linkId],
    );

  const renderGroup = (title: string, groupRows: DeclarationRow[]) => (
    <div className="mt-4">
      <h3 className="mb-2 text-[11px] font-bold uppercase tracking-wider text-slate-400">
        {title}
      </h3>
      {groupRows.length === 0 ? (
        <p className="text-xs text-brand-muted">Không có khách nào</p>
      ) : (
        <div className="overflow-hidden rounded-xl border border-slate-100">
          <table className="w-full text-sm">
            <thead>
              <tr className="bg-slate-50/60 text-left text-[11px] uppercase tracking-wider text-slate-400">
                <th className="w-10 px-3 py-2" />
                <th className="px-3 py-2">Họ tên</th>
                <th className="px-3 py-2">Quốc tịch</th>
                <th className="px-3 py-2">Phòng</th>
                <th className="px-3 py-2">Ngày đến</th>
                <th className="px-3 py-2">Kiểm tra</th>
              </tr>
            </thead>
            <tbody>
              {groupRows.map((r) => {
                const rowFindings = findingsByLink.get(r.link_id) ?? [];
                return (
                  <tr key={r.link_id} className="border-t border-slate-100">
                    <td className="px-3 py-2">
                      <input
                        type="checkbox"
                        aria-label={`Chọn ${r.full_name}`}
                        checked={selected.includes(r.link_id)}
                        onChange={() => toggle(r.link_id)}
                      />
                    </td>
                    <td className="px-3 py-2 font-semibold">{r.full_name}</td>
                    <td className="px-3 py-2 font-mono text-xs">
                      {r.nationality_iso3}
                    </td>
                    <td className="px-3 py-2">{r.room_no ?? "—"}</td>
                    <td className="px-3 py-2">{r.check_in_date}</td>
                    <td className="px-3 py-2">
                      {rowFindings.length === 0 ? (
                        <span className="text-xs text-emerald-600">Đạt</span>
                      ) : (
                        <span className="flex flex-wrap gap-1">
                          {rowFindings.map((f) => (
                            <span
                              key={`${f.code}-${f.field ?? ""}`}
                              title={f.message}
                              className={`rounded-md px-1.5 py-0.5 text-[10px] font-bold ${
                                f.severity === "blocking"
                                  ? "bg-red-50 text-red-600"
                                  : "bg-amber-50 text-amber-700"
                              }`}
                            >
                              {f.code}
                            </span>
                          ))}
                        </span>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );

  return (
    <div className="rounded-2xl bg-white p-6 shadow-soft">
      <h2 className="text-lg font-bold">Cần khai báo</h2>

      {waiting.length > 1 && (
        <div className="mt-3 flex flex-wrap gap-2">
          {waiting.map((w) => (
            <button
              key={w.id}
              type="button"
              onClick={() => setActiveId(w.id)}
              className={`rounded-full px-3 py-1 text-xs ${
                w.id === activeIdentity?.id
                  ? "bg-slate-800 text-white"
                  : "bg-slate-100 text-slate-600"
              }`}
            >
              {w.full_name}
            </button>
          ))}
        </div>
      )}

      {activeIdentity && (
        <div className="mt-4 rounded-xl border border-slate-200 bg-slate-50 p-4">
          <div className="mb-3 flex items-center justify-between gap-3">
            <p className="text-sm">
              Ghép <strong>{activeIdentity.full_name}</strong> vào một lượt lưu
              trú.
            </p>
            <button
              type="button"
              className="text-xs text-slate-400 underline hover:text-red-600"
              onClick={() =>
                handleDiscard(activeIdentity.id, activeIdentity.full_name)
              }
            >
              Bỏ ảnh này
            </button>
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label
                htmlFor="kbtt-stay"
                className="mb-1 block text-xs text-slate-600"
              >
                Ghép với khách đang ở
              </label>
              <select
                id="kbtt-stay"
                className="w-full rounded-lg border border-slate-200 bg-white px-3 py-2 text-sm"
                value={stayId}
                onChange={(e) => setStayId(e.target.value)}
              >
                {/* Mặc định rỗng: app xếp thứ tự gợi ý, người chọn (spec §7). */}
                <option value="">— Chọn lượt lưu trú —</option>
                <option value={STAY_NONE}>
                  Chưa xác định phòng — khai báo trước
                </option>
                {rankedStays.map((s) => (
                  <option key={s.stay_id} value={s.stay_id}>
                    {s.room_no ? `Phòng ${s.room_no}` : "Chưa có phòng"} ·{" "}
                    {s.guest_name ?? "—"} · {s.check_in} → {s.expected_out}
                  </option>
                ))}
              </select>
            </div>
            <div>
              <label
                htmlFor="kbtt-reason"
                className="mb-1 block text-xs text-slate-600"
              >
                Lý do lưu trú
              </label>
              <select
                id="kbtt-reason"
                className="w-full rounded-lg border border-slate-200 bg-white px-3 py-2 text-sm"
                value={stayReason}
                onChange={(e) => setStayReason(e.target.value)}
              >
                {STAY_REASONS.map((r) => (
                  <option key={r.code} value={r.code}>
                    {r.code} - {r.label}
                  </option>
                ))}
              </select>
            </div>
            {stayReason === STAY_REASON_OTHER && (
              <div className="col-span-2">
                <label
                  htmlFor="kbtt-reason-note"
                  className="mb-1 block text-xs text-slate-600"
                >
                  Nhập lý do
                </label>
                <input
                  id="kbtt-reason-note"
                  className="w-full rounded-lg border border-slate-200 bg-white px-3 py-2 text-sm"
                  value={stayReasonNote}
                  onChange={(e) => setStayReasonNote(e.target.value)}
                />
              </div>
            )}
          </div>
          <Button
            type="button"
            className="mt-3 rounded-xl"
            disabled={!canLink}
            onClick={handleLink}
          >
            {linking ? "Đang ghép..." : "Ghép khách"}
          </Button>
          <p className="mt-2 flex items-center gap-1.5 text-xs text-brand-muted">
            <AlertTriangle size={12} className="text-amber-500" />
            App chỉ xếp thứ tự theo độ giống tên, không tự chọn. Chỉ hiện phòng
            đang có khách; khách chưa check-in thì chọn "Chưa xác định phòng".
          </p>
        </div>
      )}

      {renderGroup("Khách nước ngoài (XML)", foreignRows)}
      {renderGroup("Khách Việt Nam (XLSX)", vnRows)}
    </div>
  );
}
