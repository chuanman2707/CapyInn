import { useState, useEffect, useRef, type ReactNode } from "react";
import { useHotelStore } from "@/stores/useHotelStore";
import UnifiedRoomCard from "@/components/UnifiedRoomCard";
import RoomDrawer from "@/components/RoomDrawer";
import { BedDouble, Users, AlertTriangle } from "lucide-react";

export default function Rooms() {
    const { rooms, fetchRooms } = useHotelStore();
    const [activeFloor, setActiveFloor] = useState<number | null>(null);
    const [drawerRoomId, setDrawerRoomId] = useState<string | null>(null);

    useEffect(() => { fetchRooms(); }, []);

    const floors = [...new Set(rooms.map((r) => r.floor))].sort();

    const initialized = useRef(false);
    useEffect(() => {
        if (floors.length > 0 && !initialized.current) {
            initialized.current = true;
            setActiveFloor(floors[0]);
        }
    }, [floors.length]);

    const filteredRooms = activeFloor !== null ? rooms.filter((r) => r.floor === activeFloor) : rooms;

    const stats = {
        vacant: rooms.filter((r) => r.status === "vacant").length,
        occupied: rooms.filter((r) => r.status === "occupied").length,
        booked: rooms.filter((r) => r.status === "booked").length,
    };

    const handleDrawerClose = () => {
        setDrawerRoomId(null);
        fetchRooms();
    };

    return (
        <div className="flex flex-col gap-6">

            {/* Floor Tab Bar */}
            <div className="flex items-center gap-2">
                <button onClick={() => setActiveFloor(null)} className={`px-5 py-2 rounded-xl text-sm font-semibold transition-all cursor-pointer ${activeFloor === null ? "bg-brand-primary text-white shadow-soft" : "bg-white text-brand-muted hover:bg-slate-50 border border-slate-100"}`}>
                    Tất cả
                </button>
                {floors.map((f) => (
                    <button key={f} onClick={() => setActiveFloor(f)} className={`px-5 py-2 rounded-xl text-sm font-semibold transition-all cursor-pointer ${activeFloor === f ? "bg-brand-primary text-white shadow-soft" : "bg-white text-brand-muted hover:bg-slate-50 border border-slate-100"}`}>
                        Tầng {f}
                    </button>
                ))}
            </div>

            {/* Stats Bar */}
            <div className="flex items-center gap-4 bg-white rounded-2xl p-4 shadow-soft border border-slate-100">
                <StatPill icon={<BedDouble size={14} />} label="Trống" count={stats.vacant} color="text-emerald-600 bg-emerald-50" />
                <StatPill icon={<Users size={14} />} label="Có khách" count={stats.occupied} color="text-blue-600 bg-blue-50" />
                <StatPill icon={<AlertTriangle size={14} />} label="Đặt trước" count={stats.booked} color="text-purple-600 bg-purple-50" />
                <div className="ml-auto text-sm text-brand-muted font-medium">
                    Tổng: <span className="font-bold text-brand-text">{rooms.length}</span> phòng
                </div>
            </div>

            {/* Room Grid */}
            {activeFloor === null ? (
                floors.map((f) => {
                    const floorRooms = rooms.filter((r) => r.floor === f);
                    return (
                        <div key={f}>
                            <h3 className="text-sm font-bold text-brand-muted uppercase tracking-wider mb-3 ml-1">Tầng {f}</h3>
                            <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-4">
                                {floorRooms.map((room) => (
                                    <UnifiedRoomCard key={room.id} room={room} onOpenDrawer={setDrawerRoomId} />
                                ))}
                            </div>
                        </div>
                    );
                })
            ) : (
                <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-4">
                    {filteredRooms.map((room) => (
                        <UnifiedRoomCard key={room.id} room={room} onOpenDrawer={setDrawerRoomId} />
                    ))}
                </div>
            )}

            {/* Room Drawer */}
            <RoomDrawer open={!!drawerRoomId} onClose={handleDrawerClose} roomId={drawerRoomId} />
        </div>
    );
}

function StatPill({ icon, label, count, color }: { icon: ReactNode; label: string; count: number; color: string }) {
    return (
        <div className={`flex items-center gap-2 px-3 py-1.5 rounded-lg ${color}`}>
            {icon}
            <span className="text-xs font-semibold">{count} {label}</span>
        </div>
    );
}
