"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

/// 通用通知邮箱编辑控件 — Tag/Chip 风格。
///
/// 行为约定:
///   * 受控组件 (`value` + `onChange`),父组件持有真值
///   * 输入框回车/逗号/分号/点击「+」加入到列表
///   * 每个 chip 旁边一个 × 删除
///   * 自动去重 + trim;格式校验在父组件保存时做(避免每次输入 noisy)
///
/// 不做的事:
///   * 不发请求(父组件决定何时保存)
///   * 不做严格 RFC 邮箱校验(服务端兜底,这里只做最低限度)
export function KolEmailListEditor({
  value,
  onChange,
  disabled,
  placeholder = "回车添加邮箱",
}: {
  value: string[];
  onChange: (next: string[]) => void;
  disabled?: boolean;
  placeholder?: string;
}) {
  const [draft, setDraft] = useState("");

  const commit = (raw: string) => {
    const t = raw.trim().replace(/[,;]+$/, "").trim();
    if (!t) return;
    if (value.some((v) => v.toLowerCase() === t.toLowerCase())) {
      // 已存在,只清输入
      setDraft("");
      return;
    }
    // 极简校验:必须含 @ 且 @ 后含 .
    const at = t.indexOf("@");
    if (at <= 0 || t.indexOf(".", at) === -1) {
      // 不合法 — 留在输入框让用户改
      return;
    }
    onChange([...value, t]);
    setDraft("");
  };

  const remove = (idx: number) => {
    onChange(value.filter((_, i) => i !== idx));
  };

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap gap-1.5 min-h-[28px]">
        {value.length === 0 && (
          <span className="text-xs text-muted-foreground italic">
            (尚未添加任何邮箱 — 不会收到任何通知)
          </span>
        )}
        {value.map((email, idx) => (
          <span
            key={`${email}-${idx}`}
            className="inline-flex items-center gap-1 rounded-full border border-border bg-muted px-2 py-0.5 text-xs"
            title={email}
          >
            <span className="max-w-[200px] truncate">{email}</span>
            <button
              type="button"
              onClick={() => remove(idx)}
              disabled={disabled}
              className="text-muted-foreground hover:text-destructive disabled:opacity-50 cursor-pointer ml-0.5 leading-none"
              aria-label={`删除 ${email}`}
            >
              ×
            </button>
          </span>
        ))}
      </div>
      <div className="flex gap-2">
        <Input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === ",") {
              e.preventDefault();
              commit(draft);
            } else if (e.key === "Backspace" && draft === "" && value.length > 0) {
              // Backspace at empty → quick-remove last
              remove(value.length - 1);
            }
          }}
          placeholder={placeholder}
          disabled={disabled}
          type="email"
        />
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => commit(draft)}
          disabled={disabled || draft.trim().length === 0}
        >
          添加
        </Button>
      </div>
      <p className="text-[11px] text-muted-foreground">
        每个邮箱用回车或逗号分隔。每条通知会同时发给列表中的所有邮箱。
      </p>
    </div>
  );
}
