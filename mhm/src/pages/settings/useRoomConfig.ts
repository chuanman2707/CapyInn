import { useEffect, useState } from "react";
import { toast } from "sonner";
import type { ConfigurableRoom, RoomTypeItem } from "@/types";
import { formatAppError } from "@/lib/appError";
import { invokeCommand } from "@/lib/invokeCommand";

export interface RoomFormValues {
    id: string;
    name: string;
    room_type: string;
    floor: number;
    has_balcony: boolean;
    base_price: number;
    max_guests: number;
    extra_person_fee: number;
}

const EMPTY_FORM: RoomFormValues = {
    id: "",
    name: "",
    room_type: "",
    floor: 1,
    has_balcony: false,
    base_price: 300000,
    max_guests: 2,
    extra_person_fee: 100000,
};

export default function useRoomConfig() {
    const [rooms, setRooms] = useState<ConfigurableRoom[]>([]);
    const [roomTypes, setRoomTypes] = useState<RoomTypeItem[]>([]);
    const [newTypeName, setNewTypeName] = useState("");
    const [showRoomForm, setShowRoomForm] = useState(false);
    const [editingRoom, setEditingRoom] = useState<ConfigurableRoom | null>(null);
    const [form, setForm] = useState<RoomFormValues>(EMPTY_FORM);

    const loadData = () => {
        invokeCommand<ConfigurableRoom[]>("get_rooms").then(setRooms).catch(() => { });
        invokeCommand<RoomTypeItem[]>("get_room_types").then(setRoomTypes).catch(() => { });
    };

    useEffect(loadData, []);

    const resetForm = () => {
        setForm({ ...EMPTY_FORM, room_type: roomTypes[0]?.name || "" });
        setShowRoomForm(false);
        setEditingRoom(null);
    };

    const handleAddType = async () => {
        const trimmedTypeName = newTypeName.trim();
        if (!trimmedTypeName) return;
        try {
            await invokeCommand("create_room_type", { req: { name: trimmedTypeName } });
            toast.success(`Đã tạo loại phòng "${trimmedTypeName}"`);
            setNewTypeName("");
            loadData();
        } catch (error) {
            toast.error(formatAppError(error));
        }
    };

    const handleDeleteType = async (roomTypeId: string) => {
        try {
            await invokeCommand("delete_room_type", { roomTypeId });
            toast.success("Đã xóa loại phòng");
            loadData();
        } catch (error) {
            toast.error(formatAppError(error));
        }
    };

    const openEdit = (room: ConfigurableRoom) => {
        setEditingRoom(room);
        setForm({
            id: room.id,
            name: room.name,
            room_type: room.type,
            floor: room.floor,
            has_balcony: room.has_balcony,
            base_price: room.base_price,
            max_guests: room.max_guests,
            extra_person_fee: room.extra_person_fee,
        });
        setShowRoomForm(true);
    };

    const openAdd = () => {
        setEditingRoom(null);
        setForm({ ...EMPTY_FORM, room_type: roomTypes[0]?.name || "" });
        setShowRoomForm(true);
    };

    const handleSaveRoom = async () => {
        if (!form.id || !form.name || !form.room_type) {
            toast.error("Vui lòng điền đầy đủ thông tin");
            return;
        }

        try {
            if (editingRoom) {
                const updated = await invokeCommand<ConfigurableRoom>("update_room", {
                    req: {
                        room_id: editingRoom.id,
                        name: form.name,
                        room_type: form.room_type,
                        floor: form.floor,
                        has_balcony: form.has_balcony,
                        base_price: form.base_price,
                        max_guests: form.max_guests,
                        extra_person_fee: form.extra_person_fee,
                    },
                });
                setRooms((prev) => prev.map((room) => (room.id === updated.id ? updated : room)));
                toast.success("Đã cập nhật phòng!");
            } else {
                const created = await invokeCommand<ConfigurableRoom>("create_room", {
                    req: {
                        id: form.id,
                        name: form.name,
                        room_type: form.room_type,
                        floor: form.floor,
                        has_balcony: form.has_balcony,
                        base_price: form.base_price,
                        max_guests: form.max_guests,
                        extra_person_fee: form.extra_person_fee,
                    },
                });
                setRooms((prev) => [...prev, created]);
                toast.success(`Đã tạo phòng "${created.name}"!`);
            }
            resetForm();
        } catch (error) {
            toast.error(formatAppError(error));
        }
    };

    const handleDeleteRoom = async (roomId: string) => {
        if (!confirm(`Xác nhận xóa phòng ${roomId}?`)) return;
        try {
            await invokeCommand("delete_room", { roomId });
            setRooms((prev) => prev.filter((room) => room.id !== roomId));
            toast.success("Đã xóa phòng");
        } catch (error) {
            toast.error(formatAppError(error));
        }
    };

    return {
        rooms,
        roomTypes,
        newTypeName,
        setNewTypeName,
        showRoomForm,
        editingRoom,
        form,
        setForm,
        resetForm,
        handleAddType,
        handleDeleteType,
        openEdit,
        openAdd,
        handleSaveRoom,
        handleDeleteRoom,
    };
}
