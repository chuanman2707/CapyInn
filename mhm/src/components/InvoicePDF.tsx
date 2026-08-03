import {
    Document,
    Page,
    Text,
    View,
    StyleSheet,
    Font,
} from "@react-pdf/renderer";
import { fmtMoney, fmtDateShort } from "@/lib/format";
import type { GroupInvoiceData } from "@/types";

import BeVietnamProRegular from "@/assets/fonts/BeVietnamPro-Regular.ttf";
import BeVietnamProBold from "@/assets/fonts/BeVietnamPro-Bold.ttf";

Font.register({
    family: "BeVietnamPro",
    fonts: [
        { src: BeVietnamProRegular, fontWeight: 400 },
        { src: BeVietnamProBold, fontWeight: 700 },
    ],
});

export interface PricingLine {
    label: string;
    amount: number;
}

export interface InvoiceData {
    id: string;
    invoice_number: string;
    booking_id: string;
    hotel_name: string;
    hotel_address: string;
    hotel_phone: string;
    guest_name: string;
    guest_phone: string | null;
    room_name: string;
    room_type: string;
    check_in: string;
    check_out: string;
    nights: number;
    pricing_breakdown: PricingLine[];
    subtotal: number;
    deposit_amount: number;
    total: number;
    balance_due: number;
    policy_text: string | null;
    /** The booking's own notes — internal front-desk shorthand. NOT rendered. */
    notes: string | null;
    /** Guest-facing settlement wording, set only on a per-room split invoice. */
    settlement_note: string | null;
    status: string;
    created_at: string;
}

export type { GroupInvoiceData };

const navy = "#1B2A4A";
const navyLight = "#2D4373";
const gold = "#C5A55A";
const gray100 = "#F8F9FA";
const gray300 = "#DEE2E6";
const gray600 = "#6C757D";
const gray800 = "#343A40";

const s = StyleSheet.create({
    page: {
        fontFamily: "BeVietnamPro",
        fontSize: 10,
        color: gray800,
        padding: 40,
        backgroundColor: "#FFFFFF",
    },
    header: {
        backgroundColor: navy,
        borderRadius: 6,
        padding: 20,
        marginBottom: 24,
    },
    hotelName: {
        fontSize: 18,
        fontWeight: 700,
        color: "#FFFFFF",
        marginBottom: 4,
    },
    hotelSub: {
        fontSize: 9,
        color: "#B0BEC5",
        letterSpacing: 0.3,
    },
    titleRow: {
        flexDirection: "row",
        justifyContent: "space-between",
        alignItems: "center",
        marginBottom: 20,
        paddingBottom: 12,
        borderBottom: `1.5px solid ${gold}`,
    },
    invoiceTitle: {
        fontSize: 16,
        fontWeight: 700,
        color: navy,
        letterSpacing: 1,
    },
    invoiceMeta: {
        textAlign: "right" as const,
    },
    invoiceNumber: {
        fontSize: 11,
        fontWeight: 600,
        color: navy,
    },
    invoiceDate: {
        fontSize: 9,
        color: gray600,
        marginTop: 2,
    },
    infoRow: {
        flexDirection: "row",
        marginBottom: 16,
    },
    infoBox: {
        flex: 1,
        backgroundColor: gray100,
        borderRadius: 4,
        padding: 12,
        marginRight: 8,
    },
    infoBoxLast: {
        flex: 1,
        backgroundColor: gray100,
        borderRadius: 4,
        padding: 12,
        marginRight: 0,
    },
    infoLabel: {
        fontSize: 8,
        fontWeight: 600,
        color: gray600,
        textTransform: "uppercase" as const,
        letterSpacing: 0.8,
        marginBottom: 6,
    },
    infoValue: {
        fontSize: 10,
        fontWeight: 600,
        color: gray800,
        marginBottom: 2,
    },
    infoSub: {
        fontSize: 9,
        color: gray600,
    },
    detailTable: {
        marginBottom: 16,
    },
    detailRow: {
        flexDirection: "row",
        borderBottom: `0.5px solid ${gray300}`,
        paddingVertical: 6,
    },
    detailLabel: {
        width: 100,
        fontSize: 9,
        color: gray600,
        fontWeight: 600,
    },
    detailValue: {
        flex: 1,
        fontSize: 10,
        color: gray800,
    },
    pricingHeader: {
        backgroundColor: navyLight,
        borderRadius: 4,
        padding: 8,
        marginBottom: 8,
    },
    pricingTitle: {
        fontSize: 10,
        fontWeight: 700,
        color: "#FFFFFF",
        letterSpacing: 0.5,
    },
    pricingLine: {
        flexDirection: "row",
        justifyContent: "space-between",
        paddingVertical: 5,
        paddingHorizontal: 4,
        borderBottom: `0.5px solid ${gray300}`,
    },
    pricingLabel: {
        fontSize: 10,
        color: gray800,
    },
    pricingAmount: {
        fontSize: 10,
        fontWeight: 600,
        color: gray800,
    },
    totalSection: {
        marginTop: 8,
        borderTop: `1.5px solid ${navy}`,
        paddingTop: 8,
    },
    totalRow: {
        flexDirection: "row",
        justifyContent: "space-between",
        paddingVertical: 3,
        paddingHorizontal: 4,
    },
    totalLabel: {
        fontSize: 10,
        color: gray600,
    },
    totalValue: {
        fontSize: 10,
        fontWeight: 600,
        color: gray800,
    },
    grandTotalRow: {
        flexDirection: "row",
        justifyContent: "space-between",
        backgroundColor: navy,
        borderRadius: 4,
        padding: 10,
        marginTop: 6,
    },
    grandTotalLabel: {
        fontSize: 12,
        fontWeight: 700,
        color: "#FFFFFF",
    },
    grandTotalValue: {
        fontSize: 12,
        fontWeight: 700,
        color: gold,
    },
    policyBox: {
        marginTop: 20,
        padding: 12,
        backgroundColor: gray100,
        borderRadius: 4,
        borderLeft: `3px solid ${gold}`,
    },
    policyTitle: {
        fontSize: 9,
        fontWeight: 700,
        color: navy,
        marginBottom: 6,
        letterSpacing: 0.5,
    },
    policyText: {
        fontSize: 8,
        color: gray600,
        lineHeight: 1.6,
    },
    notesBox: {
        marginTop: 16,
        padding: 12,
        backgroundColor: gray100,
        borderRadius: 4,
        borderLeft: `3px solid ${navyLight}`,
    },
    notesTitle: {
        fontSize: 9,
        fontWeight: 700,
        color: navy,
        marginBottom: 6,
        letterSpacing: 0.5,
    },
    notesText: {
        fontSize: 8,
        color: gray600,
        lineHeight: 1.6,
    },
    footer: {
        position: "absolute",
        bottom: 30,
        left: 40,
        right: 40,
        textAlign: "center",
        fontSize: 8,
        color: gray600,
        borderTop: `0.5px solid ${gray300}`,
        paddingTop: 8,
    },
    // ── Group-mode table styles ──
    sectionHeader: {
        backgroundColor: navyLight,
        borderRadius: 4,
        padding: 8,
        marginBottom: 4,
        marginTop: 12,
    },
    sectionTitle: {
        fontSize: 10,
        fontWeight: 700,
        color: "#FFFFFF",
        letterSpacing: 0.5,
    },
    tableHeaderRow: {
        flexDirection: "row",
        borderBottom: `1px solid ${gray300}`,
        paddingVertical: 5,
        paddingHorizontal: 4,
        backgroundColor: gray100,
    },
    tableRow: {
        flexDirection: "row",
        borderBottom: `0.5px solid ${gray300}`,
        paddingVertical: 5,
        paddingHorizontal: 4,
    },
    cellRoom: { width: 80, fontSize: 9, fontWeight: 600 },
    cellGuest: { flex: 1, fontSize: 9 },
    cellNights: { width: 40, fontSize: 9, textAlign: "center" as const },
    cellPrice: { width: 80, fontSize: 9, textAlign: "right" as const },
    cellTotal: { width: 80, fontSize: 9, fontWeight: 600, textAlign: "right" as const },
    cellSvcName: { flex: 1, fontSize: 9 },
    cellSvcQty: { width: 30, fontSize: 9, textAlign: "center" as const },
    cellSvcPrice: { width: 80, fontSize: 9, textAlign: "right" as const },
    cellSvcTotal: { width: 80, fontSize: 9, fontWeight: 600, textAlign: "right" as const },
    subtotalRow: {
        flexDirection: "row",
        justifyContent: "flex-end",
        paddingVertical: 4,
        paddingHorizontal: 4,
        borderTop: `1px solid ${gray300}`,
    },
    subtotalLabel: {
        fontSize: 9,
        fontWeight: 600,
        color: gray600,
        marginRight: 12,
    },
    subtotalValue: {
        fontSize: 10,
        fontWeight: 700,
        color: gray800,
        width: 80,
        textAlign: "right" as const,
    },
});

// ── Props ──

interface InvoicePDFProps {
    data?: InvoiceData;
    groupData?: GroupInvoiceData;
}

export default function InvoicePDF({ data, groupData }: InvoicePDFProps) {
    const isGroup = !!groupData;

    // Derive shared hotel info
    const hotelName = isGroup ? groupData.hotel_name : data!.hotel_name;
    const hotelAddress = isGroup ? groupData.hotel_address : data!.hotel_address;
    const hotelPhone = isGroup ? groupData.hotel_phone : data!.hotel_phone;
    // Trimmed up front so a blank-but-not-null value renders nothing rather
    // than an empty "GHI CHÚ" box. Deliberately `settlement_note`, never
    // `notes`: `notes` copies the booking's internal front-desk shorthand
    // ("cọc 600k", "Agoda thanh toan") and must not be printed for a guest.
    const settlementNote = isGroup ? "" : (data!.settlement_note ?? "").trim();

    return (
        <Document>
            <Page size="A4" style={s.page}>
                {/* Header */}
                <View style={s.header}>
                    <Text style={s.hotelName}>{hotelName}</Text>
                    <Text style={s.hotelSub}>
                        {hotelAddress}
                        {hotelPhone ? ` | ${hotelPhone}` : ""}
                    </Text>
                </View>

                {/* Title row */}
                {isGroup ? (
                    <View style={s.titleRow}>
                        <Text style={s.invoiceTitle}>HÓA ĐƠN ĐOÀN</Text>
                        <Text style={s.invoiceDate}>
                            {fmtDateShort(groupData.group.created_at)}
                        </Text>
                    </View>
                ) : (
                    <View style={s.titleRow}>
                        <Text style={s.invoiceTitle}>BOOKING CONFIRMATION</Text>
                        <View style={s.invoiceMeta}>
                            <Text style={s.invoiceNumber}>{data!.invoice_number}</Text>
                            <Text style={s.invoiceDate}>{fmtDateShort(data!.created_at)}</Text>
                        </View>
                    </View>
                )}

                {/* Info boxes */}
                {isGroup ? (
                    <View style={s.infoRow}>
                        <View style={s.infoBox}>
                            <Text style={s.infoLabel}>Thông tin đoàn</Text>
                            <Text style={s.infoValue}>{groupData.group.group_name}</Text>
                            <Text style={s.infoSub}>
                                Trưởng đoàn: {groupData.group.organizer_name}
                            </Text>
                            {groupData.group.organizer_phone && (
                                <Text style={s.infoSub}>
                                    SĐT: {groupData.group.organizer_phone}
                                </Text>
                            )}
                        </View>
                        <View style={s.infoBoxLast}>
                            <Text style={s.infoLabel}>Tổng quan</Text>
                            <Text style={s.infoValue}>{groupData.rooms.length} phòng</Text>
                            <Text style={s.infoSub}>
                                {groupData.rooms[0]?.nights || 0} đêm
                            </Text>
                        </View>
                    </View>
                ) : (
                    <>
                        <View style={s.infoRow}>
                            <View style={s.infoBox}>
                                <Text style={s.infoLabel}>Guest Information</Text>
                                <Text style={s.infoValue}>{data!.guest_name}</Text>
                                {data!.guest_phone && (
                                    <Text style={s.infoSub}>Phone: {data!.guest_phone}</Text>
                                )}
                            </View>
                            <View style={s.infoBox}>
                                <Text style={s.infoLabel}>Room Details</Text>
                                <Text style={s.infoValue}>
                                    {data!.room_name} — {data!.room_type}
                                </Text>
                                <Text style={s.infoSub}>{data!.nights} night(s)</Text>
                            </View>
                        </View>

                        {/* Room detail dates */}
                        <View style={s.detailTable}>
                            <View style={s.detailRow}>
                                <Text style={s.detailLabel}>Check-in</Text>
                                <Text style={s.detailValue}>
                                    {fmtDateShort(data!.check_in)}
                                </Text>
                            </View>
                            <View style={s.detailRow}>
                                <Text style={s.detailLabel}>Check-out</Text>
                                <Text style={s.detailValue}>
                                    {fmtDateShort(data!.check_out)}
                                </Text>
                            </View>
                        </View>
                    </>
                )}

                {/* Body — single vs group */}
                {isGroup ? (
                    <>
                        {/* Room Table */}
                        <View style={s.sectionHeader}>
                            <Text style={s.sectionTitle}>CHI TIẾT PHÒNG</Text>
                        </View>
                        <View style={s.tableHeaderRow}>
                            <Text style={s.cellRoom}>Phòng</Text>
                            <Text style={s.cellGuest}>Khách</Text>
                            <Text style={s.cellNights}>Đêm</Text>
                            <Text style={s.cellPrice}>Giá/đêm</Text>
                            <Text style={s.cellTotal}>Thành tiền</Text>
                        </View>
                        {groupData.rooms.map((room, i) => (
                            <View key={i} style={s.tableRow}>
                                <Text style={s.cellRoom}>{room.room_name}</Text>
                                <Text style={s.cellGuest}>{room.guest_name}</Text>
                                <Text style={s.cellNights}>{room.nights}</Text>
                                <Text style={s.cellPrice}>
                                    {fmtMoney(room.price_per_night)}
                                </Text>
                                <Text style={s.cellTotal}>{fmtMoney(room.total)}</Text>
                            </View>
                        ))}
                        <View style={s.subtotalRow}>
                            <Text style={s.subtotalLabel}>Tổng phòng:</Text>
                            <Text style={s.subtotalValue}>
                                {fmtMoney(groupData.subtotal_rooms)}
                            </Text>
                        </View>

                        {/* Services */}
                        {groupData.services.length > 0 && (
                            <>
                                <View style={s.sectionHeader}>
                                    <Text style={s.sectionTitle}>DỊCH VỤ KÈM</Text>
                                </View>
                                <View style={s.tableHeaderRow}>
                                    <Text style={s.cellSvcName}>Dịch vụ</Text>
                                    <Text style={s.cellSvcQty}>SL</Text>
                                    <Text style={s.cellSvcPrice}>Đơn giá</Text>
                                    <Text style={s.cellSvcTotal}>Thành tiền</Text>
                                </View>
                                {groupData.services.map((svc, i) => (
                                    <View key={i} style={s.tableRow}>
                                        <Text style={s.cellSvcName}>{svc.name}</Text>
                                        <Text style={s.cellSvcQty}>{svc.quantity}</Text>
                                        <Text style={s.cellSvcPrice}>
                                            {fmtMoney(svc.unit_price)}
                                        </Text>
                                        <Text style={s.cellSvcTotal}>
                                            {fmtMoney(svc.total_price)}
                                        </Text>
                                    </View>
                                ))}
                                <View style={s.subtotalRow}>
                                    <Text style={s.subtotalLabel}>Tổng dịch vụ:</Text>
                                    <Text style={s.subtotalValue}>
                                        {fmtMoney(groupData.subtotal_services)}
                                    </Text>
                                </View>
                            </>
                        )}

                        {/* Group totals */}
                        <View style={{ ...s.totalSection, marginTop: 16 }}>
                            <View style={s.totalRow}>
                                <Text style={s.totalLabel}>Đã thanh toán</Text>
                                <Text style={{ ...s.totalValue, color: "#059669" }}>
                                    {fmtMoney(groupData.paid_amount)}
                                </Text>
                            </View>
                            <View style={s.grandTotalRow}>
                                <Text style={s.grandTotalLabel}>CÒN LẠI</Text>
                                <Text style={s.grandTotalValue}>
                                    {fmtMoney(groupData.balance_due)}
                                </Text>
                            </View>
                        </View>
                    </>
                ) : (
                    <>
                        {/* Single pricing breakdown */}
                        <View style={s.pricingHeader}>
                            <Text style={s.pricingTitle}>PRICE BREAKDOWN</Text>
                        </View>
                        {data!.pricing_breakdown.map((line, i) => (
                            <View key={i} style={s.pricingLine}>
                                <Text style={s.pricingLabel}>{line.label}</Text>
                                <Text style={s.pricingAmount}>{fmtMoney(line.amount)}</Text>
                            </View>
                        ))}

                        {/* Single totals */}
                        <View style={s.totalSection}>
                            <View style={s.totalRow}>
                                <Text style={s.totalLabel}>Subtotal</Text>
                                <Text style={s.totalValue}>{fmtMoney(data!.total)}</Text>
                            </View>
                            {data!.deposit_amount > 0 && (
                                <View style={s.totalRow}>
                                    <Text style={s.totalLabel}>Deposit</Text>
                                    <Text style={s.totalValue}>
                                        -{fmtMoney(data!.deposit_amount)}
                                    </Text>
                                </View>
                            )}
                            <View style={s.grandTotalRow}>
                                <Text style={s.grandTotalLabel}>BALANCE DUE</Text>
                                <Text style={s.grandTotalValue}>
                                    {fmtMoney(data!.balance_due)}
                                </Text>
                            </View>
                        </View>

                        {/* Settlement note — how the total was reached, for a
                            booking whose breakdown splits per room */}
                        {settlementNote !== "" && (
                            <View style={s.notesBox}>
                                <Text style={s.notesTitle}>GHI CHÚ</Text>
                                <Text style={s.notesText}>{settlementNote}</Text>
                            </View>
                        )}

                        {/* Policy */}
                        {data!.policy_text && (
                            <View style={s.policyBox}>
                                <Text style={s.policyTitle}>POLICIES</Text>
                                <Text style={s.policyText}>{data!.policy_text}</Text>
                            </View>
                        )}
                    </>
                )}

                {/* Footer */}
                <Text style={s.footer}>
                    Thank you for choosing {hotelName}
                </Text>
            </Page>
        </Document>
    );
}
