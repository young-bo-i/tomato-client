"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { kolApi } from "../api/client";

/// Self-contained "修改密码" trigger + dialog. Renders as a small
/// button that any logged-in user can click — admins use the same
/// dialog as ordinary users (admin's "reset other users' passwords"
/// path lives in the user-management edit dialog, no old-pw prompt
/// there).
export function KolPasswordChangeButton() {
  const [open, setOpen] = useState(false);
  const [oldPw, setOldPw] = useState("");
  const [newPw, setNewPw] = useState("");
  const [confirm, setConfirm] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(false);

  const reset = () => {
    setOldPw("");
    setNewPw("");
    setConfirm("");
    setError(null);
    setDone(false);
    setSubmitting(false);
  };

  const handleOpenChange = (next: boolean) => {
    setOpen(next);
    if (!next) {
      // Defer to allow the close animation to play before clearing state.
      setTimeout(reset, 200);
    }
  };

  const handleSubmit = async () => {
    if (!oldPw) {
      setError("请输入原密码");
      return;
    }
    if (newPw.length < 6) {
      setError("新密码至少 6 位");
      return;
    }
    if (newPw === oldPw) {
      setError("新密码不能与原密码相同");
      return;
    }
    if (newPw !== confirm) {
      setError("两次输入的新密码不一致");
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      await kolApi.changeMyPassword({
        old_password: oldPw,
        new_password: newPw,
      });
      setDone(true);
      // Close after a short flash so the user sees the success state.
      setTimeout(() => handleOpenChange(false), 1500);
    } catch (e) {
      setError(e instanceof Error ? e.message : "修改失败");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <>
      <Button
        variant="ghost"
        size="sm"
        className="shrink-0"
        onClick={() => setOpen(true)}
      >
        修改密码
      </Button>
      <Dialog open={open} onOpenChange={handleOpenChange}>
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle>修改密码</DialogTitle>
          </DialogHeader>
          {done ? (
            <div className="rounded-md border border-success/40 bg-success/10 px-3 py-2 text-sm text-success">
              ✓ 密码已更新,下次登录请使用新密码
            </div>
          ) : (
            <div className="space-y-4">
              <div className="space-y-2">
                <Label>原密码</Label>
                <Input
                  type="password"
                  value={oldPw}
                  onChange={(e) => setOldPw(e.target.value)}
                  placeholder="当前登录用的密码"
                  autoFocus
                  disabled={submitting}
                  onKeyDown={(e) => e.key === "Enter" && handleSubmit()}
                />
              </div>
              <div className="space-y-2">
                <Label>新密码</Label>
                <Input
                  type="password"
                  value={newPw}
                  onChange={(e) => setNewPw(e.target.value)}
                  placeholder="至少 6 位"
                  disabled={submitting}
                  onKeyDown={(e) => e.key === "Enter" && handleSubmit()}
                />
              </div>
              <div className="space-y-2">
                <Label>确认新密码</Label>
                <Input
                  type="password"
                  value={confirm}
                  onChange={(e) => setConfirm(e.target.value)}
                  placeholder="再输一次新密码"
                  disabled={submitting}
                  onKeyDown={(e) => e.key === "Enter" && handleSubmit()}
                />
              </div>
              {error && <p className="text-sm text-destructive">{error}</p>}
            </div>
          )}
          {!done && (
            <DialogFooter>
              <Button
                variant="outline"
                onClick={() => handleOpenChange(false)}
                disabled={submitting}
              >
                取消
              </Button>
              <Button onClick={handleSubmit} disabled={submitting}>
                {submitting ? "保存中..." : "保存"}
              </Button>
            </DialogFooter>
          )}
        </DialogContent>
      </Dialog>
    </>
  );
}
