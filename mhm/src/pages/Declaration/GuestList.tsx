import { useCallback, useEffect, useMemo, useState } from "react";

import { invokeCommand } from "@/lib/invokeCommand";
import type { DeclarationFinding, DeclarationRow, StayInfo } from "@/types";

import GuestCard from "./GuestCard";

interface GuestListProps {
  reloadKey: number;
  onStateChange: (state: { rows: DeclarationRow[]; findings: DeclarationFinding[] }) => void;
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
  const [localReload, setLocalReload] = useState(0);

  const reload = useCallback(() => setLocalReload((k) => k + 1), []);

  useEffect(() => {
    invokeCommand<DeclarationRow[]>("kbtt_pending_rows")
      .then((data) => setRows(data ?? []))
      .catch(() => setRows([]));
    invokeCommand<StayInfo[]>("kbtt_list_stays")
      .then((data) => setStays(data ?? []))
      .catch(() => setStays([]));
  }, [reloadKey, localReload]);

  useEffect(() => {
    const linkIds = rows.map((r) => r.link_id);
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
  }, [rows]);

  useEffect(() => {
    onStateChange({ rows, findings });
    // onStateChange của cha không stable — cùng lý do với PendingList cũ
    // (xem PendingList.tsx: "onRowsChange intentionally omitted: parent
    // callbacks are not stable"). Không đưa vào deps để tránh chạy lại effect
    // này mỗi lần cha re-render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rows, findings]);

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
