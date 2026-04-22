import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";

import { normalizeAppError, type AppError } from "@/lib/appError";
import { invokeCommand } from "@/lib/invokeCommand";

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
