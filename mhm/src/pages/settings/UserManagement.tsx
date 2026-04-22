import { useEffect, useState } from "react";
import { Shield } from "lucide-react";
import { toast } from "sonner";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { formatAppError, normalizeAppError, type AppError } from "@/lib/appError";
import { invokeCommand } from "@/lib/invokeCommand";
import { type User } from "@/stores/useAuthStore";

export default function UserManagement() {
  const [users, setUsers] = useState<User[]>([]);
  const [loadError, setLoadError] = useState<AppError | null>(null);
  const [newName, setNewName] = useState("");
  const [newPin, setNewPin] = useState("");
  const [newRole, setNewRole] = useState("receptionist");
  const [creating, setCreating] = useState(false);

  useEffect(() => {
    let active = true;

    void (async () => {
      try {
        const loadedUsers = await invokeCommand<User[]>("list_users");
        if (active) {
          setUsers(loadedUsers);
          setLoadError(null);
        }
      } catch (error) {
        if (active) {
          const appError = normalizeAppError(error);
          setLoadError(appError);
          toast.error(formatAppError(appError));
        }
      }
    })();

    return () => {
      active = false;
    };
  }, []);

  const handleCreate = async () => {
    if (!newName || newPin.length !== 4) {
      toast.error("Tên và PIN (4 số) là bắt buộc");
      return;
    }

    setCreating(true);
    try {
      const user = await invokeCommand<User>("create_user", {
        req: { name: newName, pin: newPin, role: newRole },
      });
      setUsers((prev) => [...prev, user]);
      setNewName("");
      setNewPin("");
      setNewRole("receptionist");
      toast.success(`Đã tạo user "${user.name}"!`);
    } catch (error) {
      toast.error(formatAppError(error));
    } finally {
      setCreating(false);
    }
  };

  return (
    <div className="space-y-6">
      <div>
        <h3 className="text-lg font-bold mb-1 flex items-center gap-2">
          <Shield size={20} className="text-amber-500" />
          Quản lý Nhân viên
        </h3>
        <p className="text-sm text-brand-muted">Tạo tài khoản và phân quyền cho nhân viên (chỉ Admin)</p>
      </div>

      {loadError && (
        <div role="alert" className="rounded-xl border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700">
          {formatAppError(loadError)}
        </div>
      )}

      <div className="space-y-2">
        {users.map((user) => (
          <div key={user.id} className="flex items-center justify-between p-4 bg-slate-50 rounded-xl">
            <div className="flex items-center gap-3">
              <div
                className={`w-9 h-9 rounded-xl flex items-center justify-center text-sm font-bold ${
                  user.role === "admin" ? "bg-amber-100 text-amber-700" : "bg-blue-100 text-blue-700"
                }`}
              >
                {user.name.charAt(0).toUpperCase()}
              </div>
              <div>
                <p className="font-semibold text-sm">{user.name}</p>
                <p className="text-xs text-brand-muted capitalize">{user.role}</p>
              </div>
            </div>
            <Badge className={`${user.role === "admin" ? "bg-amber-50 text-amber-700" : "bg-blue-50 text-blue-700"} border-0 rounded-full text-[10px]`}>
              {user.role === "admin" ? "👑 Admin" : "🏨 Lễ tân"}
            </Badge>
          </div>
        ))}
      </div>

      <div className="p-5 bg-slate-50 rounded-2xl space-y-4">
        <h4 className="font-bold text-sm">Thêm nhân viên mới</h4>
        <div className="grid grid-cols-2 gap-3">
          <div>
            <Label>Tên</Label>
            <Input
              value={newName}
              onChange={(event) => setNewName(event.target.value)}
              placeholder="VD: Lễ tân Minh"
              className="mt-1.5"
            />
          </div>
          <div>
            <Label>Mã PIN (4 số)</Label>
            <Input
              value={newPin}
              onChange={(event) => setNewPin(event.target.value.replace(/\D/g, "").slice(0, 4))}
              placeholder="0000"
              maxLength={4}
              className="mt-1.5 tracking-widest text-center font-mono"
            />
          </div>
          <div>
            <Label>Vai trò</Label>
            <select
              value={newRole}
              onChange={(event) => setNewRole(event.target.value)}
              className="mt-1.5 w-full bg-white border border-slate-100 rounded-xl px-3 py-2.5 text-sm font-medium outline-none"
            >
              <option value="receptionist">Lễ tân</option>
              <option value="admin">Admin</option>
            </select>
          </div>
        </div>
        <Button
          onClick={() => void handleCreate()}
          disabled={creating || !newName || newPin.length !== 4}
          className="bg-brand-primary text-white rounded-xl"
        >
          {creating ? "Đang tạo..." : "Tạo tài khoản"}
        </Button>
      </div>
    </div>
  );
}
