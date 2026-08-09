type Locale = "vi" | "en";

const translations: Record<string, Record<Locale, string>> = {
    // Sidebar
    "nav.dashboard": { vi: "Dashboard", en: "Dashboard" },
    "nav.reservations": { vi: "Đặt phòng", en: "Reservations" },
    "nav.rooms": { vi: "Phòng", en: "Rooms" },
    "nav.guests": { vi: "Khách hàng", en: "Guests" },
    "nav.analytics": { vi: "Thống kê", en: "Analytics" },
    "nav.settings": { vi: "Cài đặt", en: "Settings" },

    // Dashboard
    "dashboard.occupied": { vi: "Có khách", en: "Occupied" },
    "dashboard.vacant": { vi: "Trống", en: "Vacant" },
    "dashboard.cleaning": { vi: "Cần dọn", en: "Need Cleaning" },
    "dashboard.revenue_today": { vi: "Doanh thu hôm nay", en: "Revenue Today" },
    "dashboard.accommodation": { vi: "Sơ đồ phòng", en: "Accommodation" },
    "dashboard.recent_bookings": { vi: "Booking gần đây", en: "Recent Bookings" },
    "dashboard.activity": { vi: "Hoạt động", en: "Activity" },
    "dashboard.expenses": { vi: "Chi phí", en: "Expenses" },
    "dashboard.view_all": { vi: "Xem tất cả", en: "View all" },

    // Guests
    "guests.total": { vi: "Tổng số khách", en: "Total Guests" },
    "guests.vip": { vi: "Khách VIP", en: "VIP Guests" },
    "guests.revenue": { vi: "Tổng doanh thu từ khách", en: "Total Guest Revenue" },
    "guests.search": { vi: "Tìm tên hoặc số CCCD...", en: "Search by name or ID..." },
    "guests.name": { vi: "Họ tên", en: "Full Name" },
    "guests.doc": { vi: "CCCD", en: "ID Number" },
    "guests.nationality": { vi: "Quốc tịch", en: "Nationality" },
    "guests.stays": { vi: "Lần ở", en: "Stays" },
    "guests.spent": { vi: "Tổng chi tiêu", en: "Total Spent" },
    "guests.last_visit": { vi: "Lần cuối", en: "Last Visit" },

    // Analytics
    "analytics.title": { vi: "Business Intelligence", en: "Business Intelligence" },
    "analytics.revenue": { vi: "Doanh thu", en: "Revenue" },
    "analytics.occupancy": { vi: "Tỷ lệ lấp đầy", en: "Occupancy Rate" },
    "analytics.daily_revenue": { vi: "Doanh thu theo ngày", en: "Daily Revenue" },
    "analytics.by_source": { vi: "Doanh thu theo nguồn", en: "Revenue by Source" },
    "analytics.top_rooms": { vi: "Top 5 phòng doanh thu cao", en: "Top 5 Rooms by Revenue" },
    "analytics.expenses": { vi: "Chi phí theo danh mục", en: "Expenses by Category" },

    // Settings
    "settings.hotel_info": { vi: "Thông tin khách sạn", en: "Hotel Information" },
    "settings.room_config": { vi: "Quản lý phòng", en: "Room Configuration" },
    "settings.checkin_rules": { vi: "Quy tắc Check-in", en: "Check-in Rules" },
    "settings.ocr": { vi: "Cấu hình OCR", en: "OCR Configuration" },
    "settings.appearance": { vi: "Giao diện", en: "Appearance" },
    "settings.data": { vi: "Dữ liệu & Sao lưu", en: "Data & Backup" },
    "settings.dark_mode": { vi: "Chế độ tối", en: "Dark Mode" },
    "settings.language": { vi: "Ngôn ngữ", en: "Language" },
    "settings.save": { vi: "Lưu thay đổi", en: "Save Changes" },
    "settings.export_csv": { vi: "Xuất dữ liệu CSV", en: "Export CSV Data" },
    "settings.backup": { vi: "Sao lưu Database", en: "Backup Database" },
    "settings.reset_danger": { vi: "Xóa toàn bộ dữ liệu", en: "Delete All Data" },

    // Common
    "common.no_data": { vi: "Chưa có dữ liệu", en: "No data yet" },
    "common.loading": { vi: "Đang tải...", en: "Loading..." },
    "common.new_guest": { vi: "+ Khách mới", en: "+ New Guest" },
    "common.scanner_ready": { vi: "Scanner sẵn sàng", en: "Scanner Ready" },
    "common.collapse": { vi: "Thu gọn", en: "Collapse" },

    // Rooms
    "rooms.all": { vi: "Tất cả", en: "All" },
    "rooms.floor": { vi: "Tầng", en: "Floor" },
    "rooms.total": { vi: "Tổng", en: "Total" },
    "rooms.vacant": { vi: "Trống", en: "Vacant" },
    "rooms.occupied": { vi: "Có khách", en: "Occupied" },
    "rooms.cleaning": { vi: "Cần dọn", en: "Need Cleaning" },
    "rooms.reserved": { vi: "Đặt trước", en: "Reserved" },

    // Toasts
    "toast.export_success": { vi: "Xuất CSV thành công!", en: "CSV export successful!" },
    "toast.room_updated": { vi: "Đã cập nhật phòng!", en: "Room updated!" },
    "toast.checkin_success": { vi: "Check-in thành công!", en: "Check-in successful!" },
    "toast.checkout_success": { vi: "Check-out thành công!", en: "Check-out successful!" },
    "toast.error": { vi: "Có lỗi xảy ra", en: "An error occurred" },
};

let currentLocale: Locale = (localStorage.getItem("locale") as Locale) || "vi";

export function setLocale(locale: Locale) {
    currentLocale = locale;
    localStorage.setItem("locale", locale);
}

export function getLocale(): Locale {
    return currentLocale;
}

export function t(key: string): string {
    const entry = translations[key];
    if (!entry) return key;
    return entry[currentLocale] || entry["vi"] || key;
}

export type { Locale };
