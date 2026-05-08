"use client";

import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { kolApi } from "../api/client";
import type { EmailSettings } from "../types";

const PASSWORD_PLACEHOLDER = "(已保存,留空保持不变)";

export function KolEmailSettingsPanel() {
  const [settings, setSettings] = useState<EmailSettings | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<{
    kind: "ok" | "err";
    text: string;
  } | null>(null);

  // Form state. Password is special: empty input + saved server-side
  // means "no change", so we track it separately and only send it on
  // submit when the user has actually typed something.
  const [smtpHost, setSmtpHost] = useState("");
  const [smtpPort, setSmtpPort] = useState(587);
  const [smtpUsername, setSmtpUsername] = useState("");
  const [smtpPasswordInput, setSmtpPasswordInput] = useState("");
  const [fromAddress, setFromAddress] = useState("");
  const [fromName, setFromName] = useState("");
  const [useTls, setUseTls] = useState(true);
  const [recipientsText, setRecipientsText] = useState("");

  const [testTo, setTestTo] = useState("");

  const load = useCallback(async () => {
    setError(null);
    try {
      const s = await kolApi.getEmailSettings();
      setSettings(s);
      setSmtpHost(s.smtp_host);
      setSmtpPort(s.smtp_port);
      setSmtpUsername(s.smtp_username);
      setSmtpPasswordInput("");
      setFromAddress(s.from_address);
      setFromName(s.from_name);
      setUseTls(s.use_tls);
      setRecipientsText(s.recipients.join("\n"));
    } catch (e) {
      setError(e instanceof Error ? e.message : "加载失败");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const parseRecipients = (raw: string): string[] => {
    return raw
      .split(/[\s,;]+/)
      .map((s) => s.trim())
      .filter((s) => s.length > 0);
  };

  const handleSave = async () => {
    setSaving(true);
    setMessage(null);
    try {
      // Only include password when user typed something. The server
      // treats `null`/missing as "preserve" — never accidentally clear
      // a stored password just by saving the form.
      const payload = {
        smtp_host: smtpHost.trim(),
        smtp_port: Number(smtpPort) || 0,
        smtp_username: smtpUsername.trim(),
        smtp_password: smtpPasswordInput.length > 0 ? smtpPasswordInput : null,
        from_address: fromAddress.trim(),
        from_name: fromName.trim(),
        use_tls: useTls,
        recipients: parseRecipients(recipientsText),
      };
      const res = await kolApi.updateEmailSettings(payload);
      if (res.ok) {
        setMessage({ kind: "ok", text: "已保存" });
        await load();
      } else {
        setMessage({ kind: "err", text: "保存失败" });
      }
    } catch (e) {
      setMessage({
        kind: "err",
        text: e instanceof Error ? e.message : "保存失败",
      });
    } finally {
      setSaving(false);
    }
  };

  const handleTest = async () => {
    setTesting(true);
    setMessage(null);
    try {
      const res = await kolApi.sendTestEmail(
        testTo.trim() ? testTo.trim() : undefined,
      );
      if (res.ok) {
        setMessage({
          kind: "ok",
          text: `测试邮件已发送至 ${res.to ?? "(默认收件人)"}`,
        });
      } else {
        setMessage({
          kind: "err",
          text: res.error ?? "发送失败",
        });
      }
    } catch (e) {
      setMessage({
        kind: "err",
        text: e instanceof Error ? e.message : "发送失败",
      });
    } finally {
      setTesting(false);
    }
  };

  if (loading && !settings) {
    return <div className="text-sm text-muted-foreground p-4">加载中...</div>;
  }

  return (
    <div className="flex flex-col gap-4 max-w-2xl">
      <div className="min-w-0">
        <h2 className="text-lg font-semibold">邮件发送服务</h2>
        <p className="text-xs text-muted-foreground">
          配置 SMTP 用于后续通知发送(收益播报、告警等)。密码留空则保持
          原值不变;若要清空密码,主动填一次空格再保存。
        </p>
      </div>

      {error && (
        <div className="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      )}

      {/* === SMTP server === */}
      <div className="rounded-md border bg-muted/30 p-4 flex flex-col gap-3">
        <div className="text-sm font-medium">SMTP 服务器</div>
        <div className="grid grid-cols-1 sm:grid-cols-3 gap-2">
          <div className="sm:col-span-2 space-y-1">
            <Label className="text-xs">主机</Label>
            <Input
              value={smtpHost}
              onChange={(e) => setSmtpHost(e.target.value)}
              placeholder="smtp.example.com"
            />
          </div>
          <div className="space-y-1">
            <Label className="text-xs">端口</Label>
            <Input
              type="number"
              value={smtpPort}
              onChange={(e) => setSmtpPort(Number(e.target.value))}
              placeholder="587"
            />
          </div>
        </div>
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          <div className="space-y-1">
            <Label className="text-xs">账号</Label>
            <Input
              value={smtpUsername}
              onChange={(e) => setSmtpUsername(e.target.value)}
              autoComplete="off"
              placeholder="user@example.com"
            />
          </div>
          <div className="space-y-1">
            <Label className="text-xs">
              密码{" "}
              {settings?.is_password_set && (
                <span className="text-success">· 已保存</span>
              )}
            </Label>
            <Input
              type="password"
              value={smtpPasswordInput}
              onChange={(e) => setSmtpPasswordInput(e.target.value)}
              autoComplete="new-password"
              placeholder={
                settings?.is_password_set ? PASSWORD_PLACEHOLDER : "密码"
              }
            />
          </div>
        </div>
        <div className="flex items-center gap-2">
          <Checkbox
            id="use-tls"
            checked={useTls}
            onCheckedChange={(c) => setUseTls(c === true)}
          />
          <Label htmlFor="use-tls" className="text-xs">
            启用 TLS (端口 465 走隐式 TLS,其他端口走 STARTTLS)
          </Label>
        </div>
      </div>

      {/* === Sender === */}
      <div className="rounded-md border bg-muted/30 p-4 flex flex-col gap-3">
        <div className="text-sm font-medium">发件人</div>
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          <div className="space-y-1">
            <Label className="text-xs">发件邮箱</Label>
            <Input
              value={fromAddress}
              onChange={(e) => setFromAddress(e.target.value)}
              placeholder="noreply@example.com"
            />
          </div>
          <div className="space-y-1">
            <Label className="text-xs">显示名称(可选)</Label>
            <Input
              value={fromName}
              onChange={(e) => setFromName(e.target.value)}
              placeholder="Tomato KOL"
            />
          </div>
        </div>
      </div>

      {/* === Recipients === */}
      <div className="rounded-md border bg-muted/30 p-4 flex flex-col gap-2">
        <div className="text-sm font-medium">默认收件人</div>
        <Label className="text-xs text-muted-foreground">
          一行一个邮箱(也可用空格、逗号分隔)。后续所有通知默认发到此列表。
        </Label>
        <textarea
          className="min-h-[100px] rounded-md border bg-background p-2 text-sm font-mono"
          value={recipientsText}
          onChange={(e) => setRecipientsText(e.target.value)}
          placeholder={"alice@example.com\nbob@example.com"}
        />
      </div>

      {/* === Actions === */}
      <div className="flex flex-wrap items-center gap-2">
        <Button onClick={() => void handleSave()} disabled={saving || testing}>
          {saving ? "保存中..." : "保存"}
        </Button>
        <Button
          variant="outline"
          onClick={() => void load()}
          disabled={saving || testing}
        >
          重置(从服务端重读)
        </Button>
      </div>

      {/* === Test === */}
      <div className="rounded-md border p-4 flex flex-col gap-2">
        <div className="text-sm font-medium">发送测试邮件</div>
        <Label className="text-xs text-muted-foreground">
          收件人为空时发送到上方默认收件人列表的第一个。修改 SMTP 配置后
          建议先保存再测试(测试用的是服务端落库后的配置)。
        </Label>
        <div className="flex flex-col sm:flex-row gap-2">
          <Input
            value={testTo}
            onChange={(e) => setTestTo(e.target.value)}
            placeholder="留空使用默认收件人"
            className="font-mono text-xs"
          />
          <Button
            variant="outline"
            onClick={() => void handleTest()}
            disabled={testing || saving}
          >
            {testing ? "发送中..." : "发送测试邮件"}
          </Button>
        </div>
      </div>

      {message && (
        <div
          className={
            message.kind === "ok"
              ? "rounded-md border border-success/40 bg-success/10 px-3 py-2 text-sm text-success"
              : "rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive"
          }
        >
          {message.text}
        </div>
      )}
    </div>
  );
}
