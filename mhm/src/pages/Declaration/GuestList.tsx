import { useCallback, useEffect, useMemo, useState } from "react";

import { invokeCommand } from "@/lib/invokeCommand";
import type { DeclarationFinding, DeclarationRow, StayInfo } from "@/types";

import GuestCard from "./GuestCard";

interface GuestListProps {
  reloadKey: number;
  onStateChange: (state: {
    rows: DeclarationRow[];
    findings: DeclarationFinding[];
    /** Lần gọi kbtt_validate gần nhất thất bại — xem FINDING I5 dưới đây. */
    checkFailed: boolean;
    /**
     * link_id của các khách đang hoạt động mà lần kiểm tra thành công gần
     * nhất KHÔNG bao trùm — hoặc vì chưa từng kiểm (vừa thả ảnh xong), hoặc
     * vì lô đang được kiểm lại. Cha dùng danh sách này để không lỡ mời xuất
     * một khách mà mình chưa biết chắc có lỗi chặn hay không.
     */
    uncheckedLinkIds: string[];
  }) => void;
}

/**
 * Danh sách khách hợp nhất (spec UX §4.1). Nguồn sự thật là DB — đổi tab rồi
 * quay lại vẫn còn. Validator tự chạy lại sau mỗi thay đổi (không còn nút
 * "Kiểm tra").
 */
export default function GuestList({ reloadKey, onStateChange }: GuestListProps) {
  const [rows, setRows] = useState<DeclarationRow[]>([]);
  const [stays, setStays] = useState<StayInfo[]>([]);
  const [findings, setFindings] = useState<DeclarationFinding[]>([]);
  const [checkFailed, setCheckFailed] = useState(false);
  // link_id đã có kết quả kbtt_validate thành công BAO TRÙM chúng — không
  // phải "đã từng nằm trong findings" (một khách sạch không sinh finding
  // nào cả, nhưng vẫn đã-được-kiểm). Thay bằng đúng tập link_ids của lần gọi
  // kbtt_validate gần nhất trả về thành công.
  const [checkedLinkIds, setCheckedLinkIds] = useState<Set<string>>(new Set());
  const [localReload, setLocalReload] = useState(0);

  const reload = useCallback(() => setLocalReload((k) => k + 1), []);

  useEffect(() => {
    // Ngăn ngừa stale response bằng cách check cancelled trước mỗi setState
    let cancelled = false;

    invokeCommand<DeclarationRow[]>("kbtt_pending_rows")
      .then((data) => {
        if (!cancelled) setRows(data ?? []);
      })
      .catch(() => {
        if (!cancelled) setRows([]);
      });

    invokeCommand<StayInfo[]>("kbtt_list_stays")
      .then((data) => {
        if (!cancelled) setStays(data ?? []);
      })
      .catch(() => {
        if (!cancelled) setStays([]);
      });

    return () => {
      cancelled = true;
    };
  }, [reloadKey, localReload]);

  // FINDING I5: kbtt_validate thất bại từng bị nuốt thành setFindings([]) —
  // trông y hệt "đã kiểm, không lỗi", nên nút xuất mời xuất cả lô mà backend
  // sẽ từ chối trắng tay. Giờ một lần gọi thất bại chỉ báo `checkFailed`,
  // KHÔNG xóa `findings`/`checkedLinkIds` cũ — dữ liệu cũ vẫn là thứ tốt
  // nhất đang có, chỉ là không còn chắc nó còn đúng.
  useEffect(() => {
    const linkIds = rows.map((r) => r.link_id);
    if (linkIds.length === 0) {
      setFindings([]);
      setCheckedLinkIds(new Set());
      setCheckFailed(false);
      return;
    }
    let cancelled = false;
    invokeCommand<DeclarationFinding[]>("kbtt_validate", { linkIds })
      .then((data) => {
        if (cancelled) return;
        setFindings(data ?? []);
        setCheckedLinkIds(new Set(linkIds));
        setCheckFailed(false);
      })
      .catch(() => {
        if (!cancelled) setCheckFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [rows]);

  // Khách chưa nằm trong tập đã-kiểm-thành-công gần nhất: hoặc vừa được thả
  // vào (chưa có link_id nào tương ứng trong checkedLinkIds), hoặc lần kiểm
  // hiện tại vẫn đang treo. checkedLinkIds chỉ được GHI ĐÈ khi có kết quả
  // thành công mới (xem effect trên) nên trong lúc một lượt validate bình
  // thường (sau khi sửa một thẻ) đang bay, các khách CŨ vẫn coi là đã kiểm —
  // không có gì để "nhấp nháy" nút xuất trên mỗi lần sửa thẻ bình thường.
  const uncheckedLinkIds = useMemo(
    () => rows.filter((r) => !checkedLinkIds.has(r.link_id)).map((r) => r.link_id),
    [rows, checkedLinkIds],
  );

  useEffect(() => {
    onStateChange({ rows, findings, checkFailed, uncheckedLinkIds });
    // onStateChange của cha không stable ("onRowsChange intentionally
    // omitted: parent callbacks are not stable"). Không đưa vào deps để
    // tránh chạy lại effect này mỗi lần cha re-render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rows, findings, checkFailed, uncheckedLinkIds]);

  const byLink = useMemo(() => {
    const map = new Map<string, DeclarationFinding[]>();
    for (const f of findings) {
      map.set(f.link_id, [...(map.get(f.link_id) ?? []), f]);
    }
    return map;
  }, [findings]);

  const active = rows.filter((r) => !r.held);
  const held = rows.filter((r) => r.held);

  return (
    <section className="rounded-2xl bg-white p-6 shadow-soft">
      <h2 className="text-lg font-bold">Chưa khai báo ({active.length})</h2>
      {active.length === 0 ? (
        <p className="mt-2 text-sm text-brand-muted">
          Không còn ai chờ khai. Thả ảnh giấy tờ vào ô trên để thêm khách.
        </p>
      ) : (
        <div className="mt-3 space-y-3">
          {active.map((r) => (
            <GuestCard
              key={r.link_id}
              row={r}
              stays={stays}
              findings={byLink.get(r.link_id) ?? []}
              onChanged={reload}
            />
          ))}
        </div>
      )}

      {held.length > 0 && (
        <details className="mt-4">
          <summary className="cursor-pointer text-sm font-semibold text-slate-500">
            Đã gác lại ({held.length})
          </summary>
          <div className="mt-3 space-y-3 opacity-80">
            {held.map((r) => (
              <GuestCard
                key={r.link_id}
                row={r}
                stays={stays}
                findings={byLink.get(r.link_id) ?? []}
                onChanged={reload}
              />
            ))}
          </div>
        </details>
      )}
    </section>
  );
}
