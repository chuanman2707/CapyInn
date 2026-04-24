import { useEffect, useState } from "react";
import { useHotelStore } from "../stores/useHotelStore";
import ActivityDetailDrawer from "@/components/ActivityDetailDrawer";
import UnifiedRoomCard from "../components/UnifiedRoomCard";
import RoomDrawer from "../components/RoomDrawer";
import StatCard from "@/components/shared/StatCard";
import EmptyState from "@/components/shared/EmptyState";
import { Area, AreaChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { Users, DoorOpen, Paintbrush, TrendingUp } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { fmtDateShort, fmtMoney, fmtNumber } from "@/lib/format";
import type { ActivityItem, BookingWithGuest, ChartDataPoint, ExpenseItem, RoomAvailability } from "@/types";

const DAY_NAMES = ["CN", "T2", "T3", "T4", "T5", "T6", "T7"];

export default function Dashboard() {
  const { rooms, stats, dashboardRefreshVersion, fetchRooms, fetchStats, setTab } = useHotelStore();
  const [activities, setActivities] = useState<ActivityItem[]>([]);
  const [recentBookings, setRecentBookings] = useState<BookingWithGuest[]>([]);
  const [expenses, setExpenses] = useState<ExpenseItem[]>([]);
  const [chartData, setChartData] = useState<ChartDataPoint[]>([]);
  const [roomAvailability, setRoomAvailability] = useState<Record<string, string | null>>({});
  const [drawerRoomId, setDrawerRoomId] = useState<string | null>(null);
  const [selectedActivity, setSelectedActivity] = useState<ActivityItem | null>(null);

  useEffect(() => {
    fetchRooms();
    fetchStats();
    invoke<ActivityItem[]>("get_recent_activity", { limit: 8 })
      .then(setActivities).catch(() => { });
    invoke<BookingWithGuest[]>("get_all_bookings", { filter: { status: "active" } })
      .then((data) => setRecentBookings(data.slice(0, 5))).catch(() => { });
    invoke<RoomAvailability[]>("get_rooms_availability", {})
      .then((data) => {
        const map: Record<string, string | null> = {};
        for (const ra of data) {
          map[ra.room.id] = ra.next_available_until || null;
        }
        setRoomAvailability(map);
      }).catch(() => { });
    const today = new Date();
    const from = new Date(today.getTime() - 30 * 24 * 60 * 60 * 1000).toISOString().split("T")[0];
    const to = today.toISOString().split("T")[0];
    invoke<{ daily_revenue: { date: string; revenue: number }[] }>("get_analytics", { period: "7d" })
      .then((data) => {
        const mapped = data.daily_revenue.map((d) => {
          const dayIdx = new Date(d.date + "T00:00:00").getDay();
          return { name: DAY_NAMES[dayIdx], revenue: d.revenue };
        });
        setChartData(mapped.length > 0 ? mapped : []);
      }).catch(() => { });
    invoke<{ category: string; amount: number; id: string; note: string | null; expense_date: string; created_at: string }[]>("get_expenses", { from, to })
      .then((data) => {
        const grouped = data.reduce<Record<string, number>>((acc, e) => {
          acc[e.category] = (acc[e.category] || 0) + e.amount;
          return acc;
        }, {});
        setExpenses(Object.entries(grouped).map(([category, amount]) => ({ category, amount })));
      }).catch(() => { });
  }, [dashboardRefreshVersion]);

  const maxExpense = Math.max(...expenses.map(e => e.amount), 1);

  const handleDrawerClose = () => {
    setDrawerRoomId(null);
    fetchRooms();
    fetchStats();
  };

  const handleActivityRoomOpen = (roomId: string) => {
    setSelectedActivity(null);
    setDrawerRoomId(roomId);
  };

  return (
    <div className="space-y-6">

      {/* 4.0 Stats Summary Row */}
      {stats && (
        <div className="grid grid-cols-4 gap-6">
          <StatCard icon={Users} label="Occupied" value={stats.occupied} sub={`/ ${stats.total_rooms}`} color="text-brand-primary" bgColor="bg-brand-primary/10" />
          <StatCard icon={DoorOpen} label="Vacant" value={stats.vacant} sub={`/ ${stats.total_rooms}`} color="text-status-vacant-text" bgColor="bg-status-vacant-bg" />
          <StatCard icon={Paintbrush} label="Need Cleaning" value={stats.cleaning} sub={`/ ${stats.total_rooms}`} color="text-status-unpaid-text" bgColor="bg-status-unpaid-bg" />
          <StatCard icon={TrendingUp} label="Revenue Today" value={fmtNumber(stats.revenue_today)} sub="VNĐ" color="text-status-partPaid-text" bgColor="bg-status-partPaid-bg" />
        </div>
      )}

      {/* 4.1 Bento Grid Layout */}
      <div className="grid grid-cols-1 lg:grid-cols-12 gap-6">

        {/* Analytics Widget (Cột 8 - Trái) */}
        <div className="lg:col-span-8 bg-white rounded-3xl shadow-soft p-6 flex flex-col h-[340px] min-w-0">
          <div className="flex items-center justify-between mb-6">
            <h2 className="text-lg font-bold text-brand-text">Analytics</h2>
            <div className="flex gap-2">
              <select className="text-sm font-medium bg-slate-50 border border-slate-100 rounded-lg px-3 py-1 outline-none">
                <option>Doanh thu</option>
              </select>
              <select className="text-sm font-medium bg-slate-50 border border-slate-100 rounded-lg px-3 py-1 outline-none">
                <option>7 ngày</option>
              </select>
            </div>
          </div>

          <div className="flex-1 min-h-0">
            <ResponsiveContainer width="100%" height="100%">
              <AreaChart data={chartData} margin={{ top: 10, right: 0, left: -20, bottom: 0 }}>
                <defs>
                  <linearGradient id="colorRevenue" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="5%" stopColor="#2563EB" stopOpacity={0.2} />
                    <stop offset="95%" stopColor="#2563EB" stopOpacity={0} />
                  </linearGradient>
                </defs>
                <XAxis dataKey="name" axisLine={false} tickLine={false} tick={{ fontSize: 12, fill: '#64748B' }} dy={10} />
                <YAxis axisLine={false} tickLine={false} tick={{ fontSize: 12, fill: '#64748B' }} tickFormatter={(v) => v >= 1000000 ? `${(v / 1000000).toFixed(0)}M` : v >= 1000 ? `${(v / 1000).toFixed(0)}K` : v} />
                <Tooltip contentStyle={{ borderRadius: '12px', border: 'none', boxShadow: '0 10px 40px -10px rgba(0,0,0,0.1)' }} labelStyle={{ fontWeight: 'bold', color: '#0F172A' }} formatter={(v) => [fmtMoney(Number(v)), 'Doanh thu']} />
                <Area type="monotone" dataKey="revenue" stroke="#2563EB" strokeWidth={3} fillOpacity={1} fill="url(#colorRevenue)" activeDot={{ r: 6, strokeWidth: 0, fill: '#2563EB' }} />
              </AreaChart>
            </ResponsiveContainer>
          </div>
        </div>

        {/* Sơ đồ 10 Phòng (Cột 4 - Phải) */}
        <div className="lg:col-span-4 bg-white rounded-3xl shadow-soft p-6 flex flex-col h-[340px] min-w-0 overflow-hidden">
          <div className="flex items-center justify-between mb-4">
            <h2 className="text-lg font-bold text-brand-text">Accommodation</h2>
            <button onClick={() => setTab('rooms')} className="text-brand-primary text-sm font-medium hover:underline cursor-pointer">Xem tất cả</button>
          </div>

          <div className="flex-1 overflow-y-auto pr-1">
            <div className="grid grid-cols-2 gap-3">
              {rooms.map((room) => (
                <UnifiedRoomCard
                  key={room.id}
                  room={room}
                  nextReservationDate={roomAvailability[room.id]}
                  onOpenDrawer={setDrawerRoomId}
                  compact
                />
              ))}
            </div>
          </div>

          <div className="mt-4 pt-4 border-t border-slate-50 flex gap-4 text-[11px] font-medium text-brand-muted">
            <div className="flex items-center gap-1.5"><div className="w-2 h-2 rounded-full bg-status-vacant-border"></div> Vacant</div>
            <div className="flex items-center gap-1.5"><div className="w-2 h-2 rounded-full bg-status-paid-border"></div> Occupied</div>
          </div>
        </div>

        {/* Recent Bookings (Cột 5 - Dưới trái) */}
        <div className="lg:col-span-5 bg-white rounded-3xl shadow-soft p-6 min-w-0">
          <div className="flex items-center justify-between mb-4">
            <h2 className="text-lg font-bold text-brand-text">Recent Bookings</h2>
            <button onClick={() => setTab('guests')} className="text-brand-primary text-sm font-medium hover:underline cursor-pointer">Xem tất cả</button>
          </div>

          <Table>
            <TableHeader>
              <TableRow className="border-b border-slate-50 hover:bg-transparent">
                <TableHead className="text-xs uppercase text-slate-400 font-semibold tracking-wider">Guest</TableHead>
                <TableHead className="text-xs uppercase text-slate-400 font-semibold tracking-wider">Status</TableHead>
                <TableHead className="text-xs uppercase text-slate-400 font-semibold tracking-wider">Room</TableHead>
                <TableHead className="text-xs uppercase text-slate-400 font-semibold tracking-wider">Check-in</TableHead>
                <TableHead className="text-xs uppercase text-slate-400 font-semibold tracking-wider text-right">Checkout</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {recentBookings.map((b) => (
                <TableRow key={b.id} className="border-b border-slate-50 hover:bg-slate-50/50 transition-colors">
                  <TableCell className="font-semibold text-brand-text py-3">{b.guest_name}</TableCell>
                  <TableCell className="py-3">
                    <Badge variant="paid" className={`border-0 rounded-md py-0.5 px-2 font-semibold ${b.paid_amount >= b.total_price ? "bg-emerald-50 text-emerald-600" : "bg-orange-50 text-orange-600"}`}>
                      {b.paid_amount >= b.total_price ? "Paid" : "Unpaid"}
                    </Badge>
                  </TableCell>
                  <TableCell className="font-semibold text-brand-text py-3">{b.room_id}</TableCell>
                  <TableCell className="text-brand-muted font-medium py-3">{fmtDateShort(b.check_in_at)}</TableCell>
                  <TableCell className="text-right text-brand-muted font-medium py-3">{fmtDateShort(b.expected_checkout)}</TableCell>
                </TableRow>
              ))}
              {recentBookings.length === 0 && (
                <TableRow>
                  <TableCell colSpan={5} className="text-center text-brand-muted py-8">Chưa có booking nào</TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        </div>

        {/* Expense Breakdown (Cột 3 - Giữa dưới) */}
        <div className="lg:col-span-3 bg-white rounded-3xl shadow-soft p-6 min-w-0">
          <div className="flex items-center justify-between mb-4">
            <h2 className="text-lg font-bold text-brand-text">Expenses</h2>
            <span className="text-xs text-brand-muted font-medium">30 ngày</span>
          </div>
          <div className="space-y-4">
            {expenses.length > 0 ? expenses.map((cat) => (
              <div key={cat.category}>
                <div className="flex items-center justify-between mb-1.5">
                  <span className="text-sm font-medium text-brand-text">{cat.category}</span>
                </div>
                <div className="h-1.5 bg-slate-100 rounded-full overflow-hidden mb-1">
                  <div className="h-full bg-brand-primary rounded-full" style={{ width: `${(cat.amount / maxExpense) * 100}%` }} />
                </div>
                <div className="flex justify-between text-[11px] text-brand-muted font-medium">
                  <span>{fmtMoney(cat.amount)}</span>
                </div>
              </div>
            )) : (
              <EmptyState message="Chưa có chi phí" />
            )}
          </div>
        </div>

        {/* Activity Feed (Cột 4 - Phải dưới) */}
        <div className="lg:col-span-4 bg-white rounded-3xl shadow-soft p-6 flex flex-col min-w-0">
          <div className="flex items-center justify-between mb-4">
            <h2 className="text-lg font-bold text-brand-text">Activity</h2>
            <span className="text-xs text-brand-muted font-medium">Recent</span>
          </div>
          <div className="flex-1 space-y-3 overflow-y-auto pr-1">
            {activities.length > 0 ? activities.map((item, i) => (
              <button
                key={`${item.kind ?? "activity"}-${item.occurred_at ?? item.time}-${i}`}
                type="button"
                onClick={() => setSelectedActivity(item)}
                className="w-full flex items-start gap-3 p-2.5 rounded-xl hover:bg-slate-50/50 transition-colors text-left cursor-pointer"
              >
                <div className={`w-8 h-8 rounded-lg flex items-center justify-center shrink-0 text-sm ${item.color}`}>
                  {item.icon}
                </div>
                <div className="min-w-0 flex-1">
                  <p className="text-sm font-medium text-brand-text leading-tight">{item.text}</p>
                  <p className="text-[11px] text-brand-muted mt-0.5">{item.time}</p>
                </div>
              </button>
            )) : (
              <EmptyState message="Chưa có hoạt động nào" />
            )}
          </div>
        </div>

      </div>

      <ActivityDetailDrawer
        open={!!selectedActivity}
        activity={selectedActivity}
        onClose={() => setSelectedActivity(null)}
        onOpenRoom={handleActivityRoomOpen}
      />

      {/* Room Drawer */}
      <RoomDrawer open={!!drawerRoomId} onClose={handleDrawerClose} roomId={drawerRoomId} />
    </div >
  );
}
