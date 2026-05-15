import { useEffect, useState } from "react";
import {
  BedDouble,
  Bot,
  Building2,
  Camera,
  Clock,
  Database,
  DollarSign,
  Palette,
  RefreshCcw,
  Users,
  Wifi,
} from "lucide-react";

import { Card } from "@/components/ui/card";
import { useExperimentalRuntimeStatus } from "@/lib/experimentalProfile";
import { useAuthStore } from "@/stores/useAuthStore";

import AppearanceSection from "./AppearanceSection";
import CheckinRulesSection from "./CheckinRulesSection";
import CeoAgentSection from "./CeoAgentSection";
import DataSection from "./DataSection";
import DiagnosticsSection from "./DiagnosticsSection";
import GatewaySection from "./GatewaySection";
import HotelInfoSection from "./HotelInfoSection";
import OcrConfigSection from "./OcrConfigSection";
import PricingSection from "./PricingSection";
import RoomConfigSection from "./RoomConfigSection";
import SoftwareUpdateSection from "./SoftwareUpdateSection";
import UserManagement from "./UserManagement";

type SettingsSectionKey =
  | "hotel"
  | "rooms"
  | "checkin"
  | "ocr"
  | "appearance"
  | "diagnostics"
  | "data"
  | "gateway"
  | "ceo-agent"
  | "updates"
  | "pricing"
  | "users";

export default function SettingsPage() {
  const { isAdmin } = useAuthStore();
  const [activeSection, setActiveSection] = useState<SettingsSectionKey>("hotel");
  const experimentalRuntime = useExperimentalRuntimeStatus(true);
  const isCurrentAdmin = isAdmin();

  const sections = [
    { key: "hotel" as const, label: "Hotel Info", icon: Building2 },
    { key: "rooms" as const, label: "Room Config", icon: BedDouble },
    { key: "checkin" as const, label: "Check-in Rules", icon: Clock },
    { key: "ocr" as const, label: "OCR Config", icon: Camera },
    { key: "appearance" as const, label: "Appearance", icon: Palette },
    { key: "diagnostics" as const, label: "Diagnostics", icon: Database },
    { key: "data" as const, label: "Data & Backup", icon: Database },
    ...(experimentalRuntime.gatewayRuntimeEnabled
      ? [{ key: "gateway" as const, label: "MCP Gateway", icon: Wifi }]
      : []),
    { key: "updates" as const, label: "Software Update", icon: RefreshCcw },
    ...(isCurrentAdmin && experimentalRuntime.agentRuntimeEnabled
      ? [{ key: "ceo-agent" as const, label: "CEO Agent", icon: Bot }]
      : []),
    ...(isCurrentAdmin
      ? [
        { key: "pricing" as const, label: "Pricing", icon: DollarSign },
        { key: "users" as const, label: "Users", icon: Users },
      ]
      : []),
  ];

  useEffect(() => {
    if (
      (activeSection === "gateway" && !experimentalRuntime.gatewayRuntimeEnabled) ||
      (
        activeSection === "ceo-agent" &&
        (!isCurrentAdmin || !experimentalRuntime.agentRuntimeEnabled)
      )
    ) {
      setActiveSection("hotel");
    }
  }, [
    activeSection,
    experimentalRuntime.agentRuntimeEnabled,
    experimentalRuntime.gatewayRuntimeEnabled,
    isCurrentAdmin,
  ]);

  return (
    <div className="flex gap-6 h-full">
      <div className="w-[200px] shrink-0">
        <nav className="flex flex-col gap-1">
          {sections.map((section) => {
            const Icon = section.icon;
            const isActive = activeSection === section.key;
            return (
              <button
                key={section.key}
                onClick={() => setActiveSection(section.key)}
                className={`flex items-center gap-3 px-4 py-2.5 rounded-xl text-sm font-medium transition-all cursor-pointer text-left ${isActive ? "bg-brand-primary/10 text-brand-primary" : "text-brand-muted hover:bg-slate-50 hover:text-brand-text"
                  }`}
              >
                <Icon size={16} />
                {section.label}
              </button>
            );
          })}
        </nav>
      </div>

      <Card className="flex-1 p-8 overflow-y-auto">
        {activeSection === "hotel" && <HotelInfoSection />}
        {activeSection === "rooms" && <RoomConfigSection />}
        {activeSection === "checkin" && <CheckinRulesSection />}
        {activeSection === "ocr" && <OcrConfigSection />}
        {activeSection === "appearance" && <AppearanceSection />}
        {activeSection === "diagnostics" && <DiagnosticsSection />}
        {activeSection === "data" && <DataSection />}
        {activeSection === "gateway" && experimentalRuntime.gatewayRuntimeEnabled && (
          <GatewaySection />
        )}
        {activeSection === "updates" && <SoftwareUpdateSection />}
        {activeSection === "ceo-agent" &&
          isCurrentAdmin &&
          experimentalRuntime.agentRuntimeEnabled && (
            <CeoAgentSection />
          )}
        {activeSection === "pricing" && isCurrentAdmin && <PricingSection />}
        {activeSection === "users" && isCurrentAdmin && <UserManagement />}
      </Card>
    </div>
  );
}
