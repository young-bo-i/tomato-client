"use client";

import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { kolApi } from "../api/client";
import { useKolAuth } from "../hooks/use-kol-auth";
import { KolEmailListEditor } from "./kol-email-list-editor";

/// 任何登录用户都能用的「我的通知设置」面板。把当前 user.notify_emails
/// 拷一份到 draft,改完点保存,服务端规范化后回吐生效列表 + refresh /me
/// 让 auth context 同步。
export function KolNotifySettingsPanel() {
  const { user, refresh } = useKolAuth();
  const [draft, setDraft] = useState<string[]>([]);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedFlash, setSavedFlash] = useState(false);

  useEffect(() => {
    if (!user) return;
    setDraft(user.notify_emails);
  }, [user]);

  if (!user) return null;

  const dirty =
    draft.length !== user.notify_emails.length ||
    draft.some((e, i) => e !== user.notify_emails[i]);

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    try {
      const res = await kolApi.updateMyNotifyEmails({ notify_emails: draft });
      // 用服务端规范化后的列表更新 draft (服务端会去重 / trim)
      setDraft(res.notify_emails);
      await refresh();
      setSavedFlash(true);
      setTimeout(() => setSavedFlash(false), 2000);
    } catch (e) {
      setError(e instanceof Error ? e.message : "保存失败");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="flex flex-col gap-6 max-w-2xl">
      <div>
        <h2 className="text-lg font-semibold">通知邮箱</h2>
        <p className="text-xs text-muted-foreground leading-relaxed mt-1">
          配置接收所有通知的邮箱。每条通知(账号掉线、收益变化、七猫月度收益等)
          都会同时发送给列表中的所有邮箱。可以填多个,适合"自己 + 同事 + 备用邮箱"
          的场景。空列表 = 不接收任何通知。
        </p>
      </div>

      {error && (
        <div className="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      )}

      <section className="rounded-lg border p-4 flex flex-col gap-4">
        <header>
          <h3 className="text-base font-semibold">收件邮箱列表</h3>
        </header>
        <KolEmailListEditor value={draft} onChange={setDraft} disabled={saving} />
        <footer className="flex items-center gap-3 pt-2 border-t">
          <Button onClick={() => void handleSave()} disabled={!dirty || saving}>
            {saving ? "保存中..." : dirty ? "保存修改" : "已保存"}
          </Button>
          {savedFlash && <span className="text-xs text-success">✓ 已应用</span>}
          {!dirty && (
            <span className="text-xs text-muted-foreground">
              当前生效:{user.notify_emails.length} 个邮箱
            </span>
          )}
        </footer>
      </section>
    </div>
  );
}
