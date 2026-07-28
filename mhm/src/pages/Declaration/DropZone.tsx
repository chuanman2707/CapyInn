import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ImagePlus, PencilLine } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { invokeCommand } from "@/lib/invokeCommand";
import type { DeclarationIdentity, DeclarationSaveOutcome, ExtractedIdentity } from "@/types";

import { declarationErrorMessage } from "./declarationError";
import IdentityCard from "./IdentityCard";
import ManualForm from "./ManualForm";
import { matchedExistingDeclarationMessage } from "./saveOutcome";

interface DropZoneProps {
  /** Gọi khi một danh tính đã được lưu, để danh sách khách tự tải lại từ DB. */
  onIdentitySaved?: () => void;
}

interface DropPayload {
  type?: string;
  paths?: string[];
}

/**
 * Khối 1 — kéo-thả ảnh giấy tờ.
 *
 * Ảnh chỉ được đọc từ đường dẫn tạm rồi bỏ. Không copy vào storage của app,
 * không log đường dẫn (spec §12.3, §12.4).
 */
export default function DropZone({ onIdentitySaved }: DropZoneProps) {
  const [cards, setCards] = useState<ExtractedIdentity[]>([]);
  const [busy, setBusy] = useState(false);
  const [manualOpen, setManualOpen] = useState(false);
  const [savingIndex, setSavingIndex] = useState<number | null>(null);
  const [lastError, setLastError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    const extract = async (paths: string[]) => {
      setBusy(true);
      for (const path of paths) {
        try {
          const res = await invokeCommand<ExtractedIdentity>(
            "kbtt_extract_from_image",
            { path },
          );
          setLastError(null);
          setCards((prev) => [...prev, res]);
        } catch (e) {
          const message = declarationErrorMessage(e);
          setLastError(message);
          toast.error(message);
        }
      }
      setBusy(false);
    };

    listen<DropPayload>("tauri://drag-drop", (event) => {
      const payload = event.payload ?? {};
      if (payload.type && payload.type !== "drop") return;
      const paths = payload.paths ?? [];
      if (paths.length === 0) return;
      void extract(paths);
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const updateCard = (index: number, identity: DeclarationIdentity) => {
    setCards((prev) =>
      prev.map((c, i) => (i === index ? { ...c, identity } : c)),
    );
  };

  const saveCard = async (index: number) => {
    const card = cards[index];
    if (!card) return;
    setSavingIndex(index);
    try {
      const outcome = await invokeCommand<DeclarationSaveOutcome>("kbtt_save_identity", {
        identity: card.identity,
        source: card.source,
        confidence: card.confidence,
      });
      // FINDING I1: khớp lại một khách đã có khai báo đang hoạt động không
      // tạo dòng nào — toast phải nói khác lần tạo mới, không thì thao tác
      // này trông y hệt một lần lưu bình thường trong khi thực ra không có
      // gì mới xảy ra.
      if (outcome.created_new_link) {
        toast.success("Đã lưu danh tính");
      } else {
        toast.info(matchedExistingDeclarationMessage(outcome.existing_location));
      }
      setCards((prev) => prev.filter((_, i) => i !== index));
      onIdentitySaved?.();
    } catch (e) {
      toast.error(declarationErrorMessage(e));
    } finally {
      setSavingIndex(null);
    }
  };

  return (
    <div className="rounded-2xl bg-white p-6 shadow-soft">
      <div className="rounded-2xl border-2 border-dashed border-slate-200 bg-slate-50/60 p-8 text-center">
        <ImagePlus size={28} className="mx-auto mb-2 text-slate-400" />
        <p className="text-sm font-semibold text-brand-text">
          Kéo ảnh giấy tờ vào đây
        </p>
        <p className="mt-1 text-xs text-brand-muted">
          Thẻ CCCD (QR) hoặc hộ chiếu (MRZ). Thả được nhiều ảnh cùng lúc. Ảnh chỉ
          được đọc rồi bỏ, không lưu vào máy.
        </p>
        {busy && (
          <p className="mt-2 text-xs text-brand-primary">Đang đọc ảnh...</p>
        )}
        {lastError && <p className="mt-2 text-xs text-red-600">{lastError}</p>}
      </div>

      <div className="mt-4 flex items-center gap-2">
        <Button
          type="button"
          variant="secondary"
          className="rounded-xl"
          onClick={() => setManualOpen((v) => !v)}
        >
          <PencilLine size={14} className="mr-1.5" />
          Nhập tay
        </Button>
        <span className="text-xs text-brand-muted">
          Dùng khi ảnh không có QR/MRZ (CMND cũ, giấy khai sinh, GPLX) hoặc đọc
          không ra.
        </span>
      </div>

      {manualOpen && (
        <div className="mt-4">
          <ManualForm
            onCancel={() => setManualOpen(false)}
            onSaved={() => {
              setManualOpen(false);
              onIdentitySaved?.();
            }}
          />
        </div>
      )}

      {cards.length > 0 && (
        <div className="mt-4 space-y-4">
          {cards.map((card, index) => (
            <IdentityCard
              key={`${card.identity.id}-${index}`}
              extracted={card}
              saving={savingIndex === index}
              onChange={(identity) => updateCard(index, identity)}
              onSave={() => void saveCard(index)}
              onDiscard={() =>
                setCards((prev) => prev.filter((_, i) => i !== index))
              }
            />
          ))}
        </div>
      )}
    </div>
  );
}
