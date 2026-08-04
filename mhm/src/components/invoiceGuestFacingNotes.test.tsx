import type { ButtonHTMLAttributes, ReactNode } from "react";
import { render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

/**
 * Both invoice renderers must print `settlement_note` and never `notes`.
 *
 * `invoices.notes` copies `bookings.notes` verbatim, and in the live database
 * that column is internal front-desk shorthand — "Agoda thanh toan",
 * "cọc 600k", scribbles about who is collecting the money. Flipping either
 * renderer from `settlement_note` back to `notes` is a one-token edit that
 * type-checks cleanly and puts all of that on a guest's PDF.
 *
 * These are real render tests rather than a source-level grep: what matters is
 * what the guest ends up reading, not which identifier the file mentions. The
 * react-pdf primitives are mocked down to plain DOM nodes so `InvoicePDF` —
 * which otherwise only ever produces a binary — can be asserted on the same
 * way as the on-screen dialog.
 */
vi.mock("@react-pdf/renderer", () => {
    const passthrough =
        (Tag: "div" | "span") =>
        ({ children }: { children?: ReactNode }) => <Tag>{children}</Tag>;
    return {
        Document: passthrough("div"),
        Page: passthrough("div"),
        View: passthrough("div"),
        Text: passthrough("span"),
        StyleSheet: { create: <T,>(styles: T) => styles },
        Font: { register: () => {} },
        pdf: () => ({ toBlob: async () => new Blob() }),
    };
});

vi.mock("@/components/ui/sheet", () => ({
    Sheet: ({ children }: { children: ReactNode }) => <div>{children}</div>,
    SheetContent: ({ children }: { children: ReactNode }) => <div>{children}</div>,
    SheetHeader: ({ children }: { children: ReactNode }) => <div>{children}</div>,
    SheetTitle: ({ children }: { children: ReactNode }) => <h2>{children}</h2>,
}));

vi.mock("@/components/ui/button", () => ({
    Button: ({ children, ...props }: ButtonHTMLAttributes<HTMLButtonElement>) => (
        <button {...props}>{children}</button>
    ),
}));

vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

import InvoicePDF, { type InvoiceData } from "./InvoicePDF";
import InvoiceDialog from "./InvoiceDialog";

/** Shaped after real rows in the hotel's database. */
const INTERNAL_NOTE = "Agoda thanh toan | cọc 600k, chị Hằng thu";
const SETTLEMENT_NOTE = "Thanh toán theo số đêm đã đặt";

function invoice(overrides: Partial<InvoiceData> = {}): InvoiceData {
    return {
        id: "invoice-1",
        invoice_number: "INV-20260802-001",
        booking_id: "booking-1",
        hotel_name: "CapyInn",
        hotel_address: "",
        hotel_phone: "",
        guest_name: "Nguyen Van A",
        guest_phone: null,
        room_name: "5B",
        room_type: "standard",
        check_in: "2026-04-15",
        check_out: "2026-04-18",
        nights: 3,
        pricing_breakdown: [
            { label: "Phòng 5B × 1 đêm", amount: 250000 },
            { label: "Phòng 2B × 2 đêm", amount: 500000 },
        ],
        subtotal: 750000,
        deposit_amount: 0,
        total: 750000,
        balance_due: 750000,
        policy_text: null,
        notes: INTERNAL_NOTE,
        settlement_note: SETTLEMENT_NOTE,
        status: "issued",
        created_at: "2026-04-18T09:00:00+07:00",
        ...overrides,
    };
}

const renderers = [
    {
        name: "InvoicePDF",
        render: (data: InvoiceData) => render(<InvoicePDF data={data} />),
    },
    {
        name: "InvoiceDialog",
        render: (data: InvoiceData) =>
            render(<InvoiceDialog open onOpenChange={() => {}} data={data} />),
    },
] as const;

describe.each(renderers)("$name", ({ render: renderInvoice }) => {
    it("in lời quyết toán trong khối GHI CHÚ", () => {
        const { container } = renderInvoice(invoice());
        expect(container.textContent).toContain(SETTLEMENT_NOTE);
        expect(container.textContent?.toUpperCase()).toContain("GHI CHÚ");
    });

    it("không bao giờ in ghi chú nội bộ của lễ tân cho khách", () => {
        const { container } = renderInvoice(invoice());
        for (const fragment of ["Agoda", "cọc 600k", "chị Hằng"]) {
            expect(container.textContent).not.toContain(fragment);
        }
    });

    it("không in khối GHI CHÚ rỗng khi không có lời quyết toán", () => {
        const { container } = renderInvoice(invoice({ settlement_note: null }));
        expect(container.textContent?.toUpperCase()).not.toContain("GHI CHÚ");
        expect(container.textContent).not.toContain("Agoda");
    });

    it("khoảng trắng thuần cũng không dựng khối GHI CHÚ", () => {
        const { container } = renderInvoice(invoice({ settlement_note: "   " }));
        expect(container.textContent?.toUpperCase()).not.toContain("GHI CHÚ");
    });
});
