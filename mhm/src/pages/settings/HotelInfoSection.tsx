import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { APP_NAME } from "@/lib/appIdentity";
import { invokeWriteCommand } from "@/lib/invokeCommand";

export default function HotelInfoSection() {
  const [hotelName, setHotelName] = useState(APP_NAME);
  const [address, setAddress] = useState("");
  const [phone, setPhone] = useState("");
  const [rating, setRating] = useState("4.8");

  useEffect(() => {
    invoke<string | null>("get_settings", { key: "hotel_info" })
      .then((value) => {
        if (!value) return;
        try {
          const data = JSON.parse(value);
          setHotelName(data.name || APP_NAME);
          setAddress(data.address || "");
          setPhone(data.phone || "");
          setRating(data.rating || "4.8");
        } catch {
          // Ignore invalid saved settings and keep defaults.
        }
      })
      .catch(() => {});
  }, []);

  const handleSave = () => {
    const value = JSON.stringify({ name: hotelName, address, phone, rating });
    invokeWriteCommand("save_settings", { key: "hotel_info", value })
      .then(() => toast.success("Đã lưu thông tin khách sạn!"))
      .catch(() => toast.error("Lỗi khi lưu!"));
  };

  return (
    <div className="space-y-6 max-w-lg">
      <div>
        <h3 className="text-lg font-bold mb-1">Thông tin khách sạn</h3>
        <p className="text-sm text-brand-muted">Cấu hình thông tin cơ bản của khách sạn</p>
      </div>

      <div className="space-y-4">
        <div>
          <Label>Tên khách sạn</Label>
          <Input value={hotelName} onChange={(event) => setHotelName(event.target.value)} className="mt-1.5" />
        </div>
        <div>
          <Label>Địa chỉ</Label>
          <Input
            value={address}
            onChange={(event) => setAddress(event.target.value)}
            placeholder="Nhập địa chỉ khách sạn..."
            className="mt-1.5"
          />
        </div>
        <div>
          <Label>Số điện thoại</Label>
          <Input
            value={phone}
            onChange={(event) => setPhone(event.target.value)}
            placeholder="0xx xxx xxxx"
            className="mt-1.5"
          />
        </div>
        <div>
          <Label>Rating hiển thị</Label>
          <Input
            type="number"
            value={rating}
            onChange={(event) => setRating(event.target.value)}
            min={0}
            max={5}
            step={0.1}
            className="mt-1.5 w-24"
          />
        </div>
        <Button className="bg-brand-primary text-white rounded-xl" onClick={handleSave}>
          Lưu thay đổi
        </Button>
      </div>
    </div>
  );
}
