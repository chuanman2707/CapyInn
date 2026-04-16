import { describe, expect, it, beforeEach } from "vitest";
import { setLocale, getLocale, t, type Locale } from "./i18n";

describe("i18n", () => {
    beforeEach(() => {
        // Reset localStorage and locale before each test
        localStorage.clear();
        setLocale("vi");
    });

    describe("locale management", () => {
        it("should return the default locale (vi)", () => {
            expect(getLocale()).toBe("vi");
        });

        it("should update the locale and store it in localStorage", () => {
            setLocale("en");
            expect(getLocale()).toBe("en");
            expect(localStorage.getItem("locale")).toBe("en");

            setLocale("vi");
            expect(getLocale()).toBe("vi");
            expect(localStorage.getItem("locale")).toBe("vi");
        });
    });

    describe("translation function (t)", () => {
        it("should return the translated string for a valid key in the default locale", () => {
            setLocale("vi");
            expect(t("nav.dashboard")).toBe("Dashboard");
            expect(t("nav.reservations")).toBe("Đặt phòng");
        });

        it("should return the translated string for a valid key when the locale is changed", () => {
            setLocale("en");
            expect(t("nav.reservations")).toBe("Reservations");
            expect(t("guests.total")).toBe("Total Guests");
        });

        it("should fall back to returning the key if it is not found", () => {
            expect(t("unknown.key")).toBe("unknown.key");
            expect(t("another.missing.key")).toBe("another.missing.key");
        });

        it("should fallback to 'vi' if current locale translation is somehow missing", () => {
            // Force a non-existent locale to test the fallback behavior
            setLocale("fr" as Locale);
            expect(t("nav.reservations")).toBe("Đặt phòng"); // Should fallback to "vi"
        });
    });
});
