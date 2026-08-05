import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";

import { normalizeAppError, type AppError } from "@/lib/appError";
import { invokeCommand } from "@/lib/invokeCommand";
// Một chiều: store trợ lý không biết gì về store xác thực, nên import này không
// tạo phụ thuộc vòng. Đường ngược lại (trợ lý tự đăng ký nghe store xác thực)
// mới là đường đẻ ra vòng, và cũng khó đọc hơn khi đi tìm "ai dọn cái gì".
import { useAssistantStore } from "@/stores/useAssistantStore";

export interface User {
    id: string;
    name: string;
    role: "admin" | "receptionist";
    active: boolean;
    created_at: string;
}

interface AuthStore {
    user: User | null;
    isAuthenticated: boolean;
    loading: boolean;
    error: AppError | null;

    login: (pin: string) => Promise<boolean>;
    logout: () => Promise<void>;
    checkSession: () => Promise<void>;
    clearError: () => void;
    isAdmin: () => boolean;
    hydrateFromBootstrap: (user: User | null) => void;
}

export const useAuthStore = create<AuthStore>((set, get) => ({
    user: null,
    isAuthenticated: false,
    loading: false,
    error: null,

    login: async (pin: string) => {
        set({ loading: true, error: null });
        try {
            const res = await invokeCommand<{ user: User }>("login", { req: { pin } });
            set({ user: res.user, isAuthenticated: true, loading: false, error: null });
            return true;
        } catch (error) {
            set({ error: normalizeAppError(error), loading: false });
            return false;
        }
    },

    logout: async () => {
        // Dọn trợ lý TRƯỚC khi gọi backend, đồng bộ: không có `await` nào giữa
        // "người dùng bấm đăng xuất" và "dữ liệu của người ấy biến khỏi bộ
        // nhớ". Đặt sau `await` thì một lệnh `logout` treo hoặc một lần sửa
        // sau này cho `logout` thoát sớm là để nguyên hội thoại, `history`
        // (transcript có tên khách và CCCD) và cả một thẻ nhận phòng còn duyệt
        // được — store zustand là singleton của module và app không reload.
        useAssistantStore.getState().resetForLogout();
        try {
            await invoke("logout");
        } catch { /* ignore */ }
        set({ user: null, isAuthenticated: false });
    },

    checkSession: async () => {
        try {
            const user = await invoke<User | null>("get_current_user");
            if (user) {
                set({ user, isAuthenticated: true });
                return;
            }
            set({ user: null, isAuthenticated: false });
        } catch {
            set({ user: null, isAuthenticated: false });
        }
    },

    clearError: () => set({ error: null }),

    isAdmin: () => get().user?.role === "admin",

    hydrateFromBootstrap: (user) =>
        set({
            user,
            isAuthenticated: Boolean(user),
            loading: false,
            error: null,
        }),
}));
