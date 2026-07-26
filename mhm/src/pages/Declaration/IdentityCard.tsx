import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { DeclarationIdentity, ExtractedIdentity } from "@/types";

import { isForeign, sourceLabel } from "./catalog";

interface IdentityCardProps {
  extracted: ExtractedIdentity;
  onChange: (identity: DeclarationIdentity) => void;
  onSave?: () => void;
  onDiscard?: () => void;
  saving?: boolean;
}

export default function IdentityCard({
  extracted,
  onChange,
  onSave,
  onDiscard,
  saving = false,
}: IdentityCardProps) {
  const id = extracted.identity;
  const foreign = isForeign(id.nationality_iso3);
  const hints = extracted.review_hints ?? [];

  return (
    <div className="rounded-2xl border border-slate-200 bg-white p-4">
      <div className="mb-3 flex items-center gap-2">
        <span className="text-xs font-semibold uppercase tracking-wider text-slate-500">
          {sourceLabel(extracted.source)}
        </span>
        {extracted.confidence === "verified" ? (
          <Badge className="border-0 rounded-md bg-emerald-50 text-emerald-700 text-[10px] px-1.5 py-0">
            ✓ Verified
          </Badge>
        ) : (
          <Badge className="border-0 rounded-md bg-amber-50 text-amber-700 text-[10px] px-1.5 py-0">
            ⚠ Cần xác nhận
          </Badge>
        )}
        {hints.length > 0 && (
          <span className="text-[11px] text-slate-500">
            Cần soi: {hints.join(", ")}
          </span>
        )}
        {onDiscard && (
          <button
            type="button"
            onClick={onDiscard}
            className="ml-auto text-xs text-slate-400 hover:text-red-500 cursor-pointer"
          >
            Bỏ ảnh này
          </button>
        )}
      </div>

      {/*
        Ảnh crop hai dòng MRZ PHẢI nằm trong cùng khối này, cạnh ô tên, không
        cuộn, không bấm mở. Dòng 1 của MRZ (họ tên) không có checksum nào bảo
        vệ — E04 chỉ có nghĩa nếu người ta thật sự nhìn hai dòng đó khi bấm
        "Xác nhận tên". Xem spec §11.
      */}
      <div
        data-mrz-review
        className="mb-3 flex flex-row items-start gap-4 rounded-xl bg-slate-50 p-3"
      >
        <div className="min-w-0 flex-1">
          <label
            htmlFor={`kbtt-name-${id.id}`}
            className="mb-1 block text-xs text-slate-600"
          >
            Họ và tên
          </label>
          <input
            id={`kbtt-name-${id.id}`}
            className="w-full rounded-lg border border-slate-200 bg-white px-3 py-2 font-mono text-sm"
            value={id.full_name}
            onChange={(e) => onChange({ ...id, full_name: e.target.value })}
          />
          {!id.name_confirmed_by_human && (
            <p className="mt-1 text-xs text-amber-700">
              Chưa xác nhận tên — dòng 1 của MRZ không có checksum bảo vệ.
            </p>
          )}
        </div>
        {extracted.crop_data_url && (
          <img
            src={extracted.crop_data_url}
            alt="Ảnh cắt hai dòng MRZ để đối chiếu"
            className="h-16 shrink-0 rounded border border-slate-200 bg-white object-contain"
          />
        )}
      </div>

      <div className="flex flex-wrap items-center gap-3">
        <Button
          type="button"
          size="sm"
          variant={id.name_confirmed_by_human ? "secondary" : "default"}
          className="rounded-xl"
          onClick={() => onChange({ ...id, name_confirmed_by_human: true })}
        >
          {id.name_confirmed_by_human ? "✓ Tên đã xác nhận" : "Xác nhận tên"}
        </Button>

        <label className="flex items-center gap-2 text-sm text-slate-600">
          <input
            type="checkbox"
            checked={id.single_token_name_ok ?? false}
            onChange={(e) =>
              onChange({ ...id, single_token_name_ok: e.target.checked })
            }
          />
          Tên trên giấy tờ chỉ có một chữ
        </label>
      </div>

      <div className="mt-3 grid grid-cols-2 gap-3">
        <div>
          <label
            htmlFor={`kbtt-dob-${id.id}`}
            className="mb-1 block text-xs text-slate-600"
          >
            Ngày sinh
          </label>
          <input
            id={`kbtt-dob-${id.id}`}
            type="date"
            className="w-full rounded-lg border border-slate-200 px-3 py-2 text-sm"
            value={id.dob ?? ""}
            onChange={(e) => onChange({ ...id, dob: e.target.value })}
          />
        </div>
        <div>
          <label
            htmlFor={`kbtt-nat-${id.id}`}
            className="mb-1 block text-xs text-slate-600"
          >
            Quốc tịch (ISO3)
          </label>
          <input
            id={`kbtt-nat-${id.id}`}
            className="w-full rounded-lg border border-slate-200 px-3 py-2 font-mono text-sm uppercase"
            value={id.nationality_iso3 ?? ""}
            onChange={(e) =>
              onChange({ ...id, nationality_iso3: e.target.value.toUpperCase() })
            }
          />
        </div>
      </div>

      {foreign && (
        <div className="mt-3">
          <label
            htmlFor={`kbtt-visa-${id.id}`}
            className="mb-1 block text-xs text-slate-600"
          >
            Thời hạn tạm trú (bắt buộc, nhập tay)
          </label>
          <input
            id={`kbtt-visa-${id.id}`}
            type="date"
            className="w-full rounded-lg border border-slate-200 px-3 py-2 text-sm"
            value={id.visa_valid_until ?? ""}
            onChange={(e) =>
              onChange({ ...id, visa_valid_until: e.target.value })
            }
          />
          <p className="mt-1 text-xs text-slate-500">
            Không phải ngày hết hạn hộ chiếu ({id.passport_expiry ?? "—"}). Lấy
            từ dấu nhập cảnh hoặc thị thực.
          </p>
        </div>
      )}

      {onSave && (
        <Button
          type="button"
          className="mt-4 w-full rounded-xl"
          disabled={saving}
          onClick={onSave}
        >
          {saving ? "Đang lưu..." : "Lưu danh tính và ghép khách"}
        </Button>
      )}
    </div>
  );
}
