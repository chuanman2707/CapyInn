import { beforeEach, describe, it } from "vitest";
import { clearMockResponses, invoke, setMockResponse } from "@test-mocks/tauri-core";
import { useHotelStore } from "@/stores/useHotelStore";

const delay = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

describe("Store Performance Baseline", () => {
    beforeEach(() => {
        clearMockResponses();
        invoke.mockClear();
        useHotelStore.setState({
            rooms: [],
            stats: null,
            loading: false,
            groups: [],
        });

        // Mock slow endpoints to simulate network latency
        setMockResponse("get_rooms", async () => {
            await delay(100);
            return [];
        });
        setMockResponse("get_dashboard_stats", async () => {
            await delay(100);
            return null;
        });
        setMockResponse("get_all_groups", async () => {
            await delay(100);
            return [];
        });
        setMockResponse("check_in", async () => {
            await delay(50);
            return null;
        });
        setMockResponse("group_checkin", async () => {
            await delay(50);
            return null;
        });
    });

    it("measures checkIn time", async () => {
        const start = performance.now();
        await useHotelStore.getState().checkIn(
            "1A",
            [{ full_name: "John Doe", doc_number: "123" }],
            1
        );
        const duration = performance.now() - start;
        console.log(`checkIn took ${duration}ms`);
        // Expected ~150ms if parallel, ~250ms if sequential
    });

    it("measures groupCheckIn time", async () => {
        const start = performance.now();
        await useHotelStore.getState().groupCheckIn({} as any);
        const duration = performance.now() - start;
        console.log(`groupCheckIn took ${duration}ms`);
        // Expected ~150ms if parallel, ~350ms if sequential
    });
});
