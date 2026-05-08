"use client";

import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { kolApi } from "../api/client";
import type { AdminSettings } from "../types";

/// Discrete contribution buckets. Each value divides 100 cleanly so
/// the server's Bresenham distribution collapses to a strict "every
/// Nth word" period — operator can predict exactly which word will be
/// admin-routed without thinking about long-run averages.
const CONTRIBUTION_OPTIONS: {
  pct: 0 | 10 | 20 | 50 | 100;
  label: string;
  cadence: string;
}[] = [
  { pct: 0, label: "0%", cadence: "禁用" },
  { pct: 10, label: "10%", cadence: "每 10 个词的第 10 个给管理员" },
  { pct: 20, label: "20%", cadence: "每 5 个词的第 5 个给管理员" },
  { pct: 50, label: "50%", cadence: "用户、管理员交替(1:1)" },
  { pct: 100, label: "100%", cadence: "全部优先给管理员" },
];

type ContributionPct = (typeof CONTRIBUTION_OPTIONS)[number]["pct"];

function isAllowedPct(n: number): n is ContributionPct {
  return CONTRIBUTION_OPTIONS.some((o) => o.pct === n);
}

/// Admin-only panel for global runtime knobs.
///
/// Currently the only knob is `admin_contribution_pct` — the share of
/// non-admin users' collected words the server redirects to the admin
/// pool. Restricted to 5 discrete buckets for predictable cadences;
/// the server validates the same set.
export function KolGlobalSettingsPanel() {
  const [settings, setSettings] = useState<AdminSettings | null>(null);
  const [draft, setDraft] = useState<ContributionPct>(0);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedFlash, setSavedFlash] = useState(false);

  const refresh = useCallback(async () => {
    setError(null);
    try {
      const s = await kolApi.getAdminSettings();
      setSettings(s);
      // Server might still hold a value from a prior schema/manual edit
      // outside our 5 buckets — fall back to nearest allowed value
      // (truncate down) so the UI stays consistent.
      setDraft(
        isAllowedPct(s.admin_contribution_pct)
          ? s.admin_contribution_pct
          : nearestAllowed(s.admin_contribution_pct),
      );
    } catch (e) {
      setError(e instanceof Error ? e.message : "加载失败");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const dirty = settings !== null && draft !== settings.admin_contribution_pct;

  const handleSave = async () => {
    if (!dirty) return;
    setSaving(true);
    setError(null);
    try {
      await kolApi.updateAdminSettings({ admin_contribution_pct: draft });
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
        <h2 className="text-lg font-semibold">全局设置</h2>
        <p className="text-xs text-muted-foreground">
          影响所有用户的服务端默认行为。修改后保存即时生效。
        </p>
      </div>

      {error && (
        <div className="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      )}

      <section className="rounded-lg border p-4 flex flex-col gap-4">
        <header className="flex flex-col gap-1">
          <h3 className="text-base font-semibold">采集词贡献度</h3>
          <p className="text-xs text-muted-foreground leading-relaxed">
            非管理员用户采集到的词,按比例自动重定向到管理员账号池。
            番茄和七猫各自维护独立的轮次计数,互不干扰。
          </p>
          <p className="text-[11px] text-muted-foreground">
            管理员池此时也满 → 回退到用户池;两边都满才丢弃。
            管理员自己采集的词不受此设置影响。
          </p>
        </header>

        <fieldset className="flex flex-col gap-2" disabled={loading || saving}>
          <legend className="text-xs font-medium text-muted-foreground mb-1">
            选择档位
          </legend>
          <div className="grid grid-cols-5 gap-2">
            {CONTRIBUTION_OPTIONS.map((opt) => {
              const selected = draft === opt.pct;
              return (
                <button
                  key={opt.pct}
                  type="button"
                  onClick={() => setDraft(opt.pct)}
                  className={`flex flex-col items-center gap-0.5 rounded-md border px-2 py-2 transition-colors ${
                    selected
                      ? "border-primary bg-primary/10 text-foreground"
                      : "border-border bg-card text-muted-foreground hover:bg-muted"
                  } disabled:opacity-50 disabled:cursor-not-allowed`}
                >
                  <span
                    className={`text-base font-mono font-semibold ${
                      selected ? "text-primary" : ""
                    }`}
                  >
                    {opt.label}
                  </span>
                </button>
              );
            })}
          </div>
          <div className="text-xs text-muted-foreground bg-muted rounded-md px-3 py-2">
            <span className="font-medium text-foreground">当前节奏:</span>{" "}
            {CONTRIBUTION_OPTIONS.find((o) => o.pct === draft)?.cadence}
          </div>
        </fieldset>

        <footer className="flex items-center gap-3 pt-2 border-t">
          <Button onClick={handleSave} disabled={!dirty || saving}>
            {saving ? "保存中..." : dirty ? "保存修改" : "已保存"}
          </Button>
          {savedFlash && (
            <span className="text-xs text-success">✓ 已应用</span>
          )}
          {settings && !dirty && (
            <span className="text-xs text-muted-foreground">
              上次更新 {new Date(settings.updated_at).toLocaleString()}
            </span>
          )}
        </footer>
      </section>
    </div>
  );
}

/** Pick the closest allowed bucket below the given pct. Used to
 * sanitize a server value that's outside our 5 buckets (e.g. legacy
 * 30% → 20%) so the UI doesn't render a "no option selected" state. */
function nearestAllowed(pct: number): ContributionPct {
  const allowed = CONTRIBUTION_OPTIONS.map((o) => o.pct);
  // Largest allowed value ≤ pct, with 0 as the floor.
  const best = allowed.filter((a) => a <= pct).sort((a, b) => b - a)[0];
  return (best ?? 0) as ContributionPct;
}
