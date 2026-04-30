import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { formatAppError } from "@/lib/appError";
import { assertNonNegativeMoneyVnd } from "@/lib/money";
import type { BootstrapStatus } from "@/types";

import { generateRoomPlan } from "./generateRoomPlan";
import WelcomeStep from "./steps/WelcomeStep";
import HotelInfoStep from "./steps/HotelInfoStep";
import RoomTypesStep from "./steps/RoomTypesStep";
import RoomLayoutStep from "./steps/RoomLayoutStep";
import AppLockStep from "./steps/AppLockStep";
import ReviewStep from "./steps/ReviewStep";
import { useOnboardingDraft } from "./useOnboardingDraft";
import type { OnboardingRoomTypeDraft } from "./types";

function trimRoomTypeName(name: string) {
  return name.trim();
}

function hasValidRoomTypes(roomTypes: OnboardingRoomTypeDraft[]) {
  if (roomTypes.length === 0) return false;

  const seen = new Set<string>();
  return roomTypes.every((roomType) => {
    const name = trimRoomTypeName(roomType.name);
    if (!name || roomType.basePrice < 0 || roomType.extraPersonFee < 0 || roomType.maxGuests < 1) {
      return false;
    }

    const normalized = name.toLowerCase();
    if (seen.has(normalized)) {
      return false;
    }

    seen.add(normalized);
    return true;
  });
}

function syncColumnAssignments(
  roomTypes: OnboardingRoomTypeDraft[],
  roomPlan: { roomsPerFloor: number; columnAssignments: string[] },
) {
  const validNames = roomTypes
    .map((roomType) => trimRoomTypeName(roomType.name))
    .filter((name) => Boolean(name));
  const fallback = validNames[0] ?? "";

  return Array.from({ length: Math.max(roomPlan.roomsPerFloor, 0) }, (_, index) => {
    const current = roomPlan.columnAssignments[index];
    return validNames.includes(current) ? current : fallback;
  });
}

export default function OnboardingWizard({ onCompleted }: { onCompleted?: (status: BootstrapStatus) => void }) {
  const [step, setStep] = useState(0);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { draft, setDraft, clearDraft } = useOnboardingDraft();

  const generatedResult = useMemo(() => generateRoomPlan({
    floors: draft.roomPlan.floors,
    roomsPerFloor: draft.roomPlan.roomsPerFloor,
    namingScheme: draft.roomPlan.namingScheme,
    columnAssignments: draft.roomPlan.columnAssignments,
    roomTypesByName: Object.fromEntries(
      draft.roomTypes.map((roomType) => [
        trimRoomTypeName(roomType.name),
        {
          basePrice: roomType.basePrice,
          maxGuests: roomType.maxGuests,
          extraPersonFee: roomType.extraPersonFee,
          defaultHasBalcony: roomType.defaultHasBalcony,
        },
      ]),
    ),
  }), [draft.roomPlan, draft.roomTypes]);

  const canContinue =
    step === 1 ? Boolean(draft.hotel.name && draft.hotel.address && draft.hotel.phone) :
    step === 2 ? hasValidRoomTypes(draft.roomTypes) :
    step === 3 ? generatedResult.error === null && draft.generatedRooms.length > 0 :
    step === 4 ? (!draft.appLock.enabled || (draft.appLock.adminName && /^\d{4}$/.test(draft.appLock.pin) && draft.appLock.pin === draft.appLock.confirmPin)) :
    true;

  async function handleComplete() {
    setSaving(true);
    setError(null);

    try {
      const status = await invoke<BootstrapStatus>("complete_onboarding", {
        req: {
          hotel: {
            name: draft.hotel.name,
            address: draft.hotel.address,
            phone: draft.hotel.phone,
            rating: draft.hotel.rating,
            default_checkin_time: draft.hotel.defaultCheckinTime,
            default_checkout_time: draft.hotel.defaultCheckoutTime,
            locale: draft.locale,
          },
          room_types: draft.roomTypes.map((roomType) => ({
            name: roomType.name,
            base_price: assertNonNegativeMoneyVnd(roomType.basePrice, "base_price"),
            max_guests: roomType.maxGuests,
            extra_person_fee: assertNonNegativeMoneyVnd(
              roomType.extraPersonFee,
              "extra_person_fee",
            ),
            default_has_balcony: roomType.defaultHasBalcony,
            bed_note: roomType.bedNote ?? null,
          })),
          rooms: draft.generatedRooms.map((room) => ({
            id: room.id,
            name: room.name,
            floor: room.floor,
            room_type_name: room.roomTypeName,
            has_balcony: room.hasBalcony,
            base_price: assertNonNegativeMoneyVnd(room.basePrice, "base_price"),
            max_guests: room.maxGuests,
            extra_person_fee: assertNonNegativeMoneyVnd(
              room.extraPersonFee,
              "extra_person_fee",
            ),
          })),
          app_lock: {
            enabled: draft.appLock.enabled,
            admin_name: draft.appLock.enabled ? draft.appLock.adminName : null,
            pin: draft.appLock.enabled ? draft.appLock.pin : null,
          },
        },
      });

      clearDraft();
      onCompleted?.(status);
    } catch (err) {
      setError(formatAppError(err));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="min-h-screen w-screen bg-brand-bg flex items-center justify-center px-6 py-10">
      <div className="w-full max-w-3xl rounded-3xl border border-slate-200 bg-white p-8 shadow-sm">
        {step === 0 && <WelcomeStep onStart={() => setStep(1)} />}
        {step === 1 && (
          <HotelInfoStep
            value={draft.hotel}
            onChange={(next) => {
              setError(null);
              setDraft((prev) => ({ ...prev, hotel: { ...prev.hotel, ...next } }));
            }}
          />
        )}
        {step === 2 && (
          <RoomTypesStep
            value={draft.roomTypes}
            onChange={(roomTypes) => {
              setError(null);
              setDraft((prev) => ({
                ...prev,
                roomTypes,
                generatedRooms: [],
                roomPlan: {
                  ...prev.roomPlan,
                  columnAssignments: syncColumnAssignments(roomTypes, prev.roomPlan),
                },
              }));
            }}
          />
        )}
        {step === 3 && (
          <RoomLayoutStep
            value={draft.roomPlan}
            roomTypes={draft.roomTypes
              .map((roomType) => trimRoomTypeName(roomType.name))
              .filter((name) => Boolean(name))}
            generated={draft.generatedRooms}
            error={error}
            onChange={(next) => {
              setError(null);
              setDraft((prev) => {
                const roomPlan = { ...prev.roomPlan, ...next };
                return {
                  ...prev,
                  roomPlan: {
                    ...roomPlan,
                    columnAssignments: syncColumnAssignments(prev.roomTypes, roomPlan),
                  },
                  generatedRooms: [],
                };
              });
            }}
            onGenerate={() => {
              if (generatedResult.error) {
                setError(generatedResult.error);
                setDraft((prev) => ({ ...prev, generatedRooms: [] }));
                return;
              }
              setError(null);
              setDraft((prev) => ({ ...prev, generatedRooms: generatedResult.rooms }));
            }}
          />
        )}
        {step === 4 && (
          <AppLockStep
            value={draft.appLock}
            onChange={(next) => {
              setError(null);
              setDraft((prev) => ({ ...prev, appLock: { ...prev.appLock, ...next } }));
            }}
          />
        )}
        {step === 5 && (
          <ReviewStep
            draft={draft}
            generated={draft.generatedRooms}
            error={error}
            saving={saving}
            onSubmit={handleComplete}
          />
        )}

        {step > 0 && step < 5 && (
          <div className="mt-8 flex items-center justify-between">
            <button
              type="button"
              onClick={() => setStep((prev) => Math.max(0, prev - 1))}
              className="rounded-xl border border-slate-300 px-4 py-3 font-medium cursor-pointer"
            >
              Quay lại
            </button>
            <button
              type="button"
              onClick={() => setStep((prev) => prev + 1)}
              disabled={!canContinue}
              className="rounded-xl bg-brand-primary px-5 py-3 text-white font-medium cursor-pointer disabled:opacity-60"
            >
              Tiếp tục
            </button>
          </div>
        )}

        {step === 5 && (
          <div className="mt-8">
            <button
              type="button"
              onClick={() => setStep(4)}
              className="rounded-xl border border-slate-300 px-4 py-3 font-medium cursor-pointer"
            >
              Quay lại
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
