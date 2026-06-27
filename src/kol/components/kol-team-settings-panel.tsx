"use client";
import { ErrorBanner } from "./shared/error-banner";

import { useCallback, useEffect, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { kolApi } from "../api/client";
import { useKolAuth } from "../hooks/use-kol-auth";
import type { SubordinateRow } from "../types";

/// Same 5-bucket discrete set as the admin contribution. Centralizing
/// the cadence labels here so the user sees the SAME wording whether
/// they're viewing the admin slider or this team panel.
const TIER2_OPTIONS: {
  pct: 0 | 10 | 20 | 50 | 100;
  label: string;
  cadence: string;
}[] = [
  { pct: 0, label: "0%", cadence: "禁用 — 下级所有词都留在他们自己的池中" },
  {
    pct: 10,
    label: "10%",
    cadence: "管理员拿走后,剩下的每 10 个词中第 10 个交给我",
  },
  {
    pct: 20,
    label: "20%",
    cadence: "管理员拿走后,剩下的每 5 个词中第 5 个交给我",
  },
  {
    pct: 50,
    label: "50%",
    cadence: "管理员拿走后,剩下的词与下级 1:1 交替,我拿一半",
  },
  {
    pct: 100,
    label: "100%",
    cadence: "管理员拿走后,剩下的全部交给我",
  },
];

type Tier2Pct = (typeof TIER2_OPTIONS)[number]["pct"];

function isAllowed(n: number): n is Tier2Pct {
  return TIER2_OPTIONS.some((o) => o.pct === n);
}
function nearestAllowed(n: number): Tier2Pct {
  const allowed = TIER2_OPTIONS.map((o) => o.pct);
  return ((allowed.filter((a) => a <= n).sort((a, b) => b - a)[0] ?? 0) as Tier2Pct);
}

/// Tier-1 user's "我的下级贡献度" panel. Lets the operator decide
/// what percentage of their tier-2 subordinates' (post-admin)
/// remaining words flow up to them. Only mounted when the auth gate
/// has confirmed `user.has_subordinates === true`.
export function KolTeamSettingsPanel() {
  const { user, refresh } = useKolAuth();
  const [draft, setDraft] = useState<Tier2Pct>(0);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedFlash, setSavedFlash] = useState(false);
  const [subs, setSubs] = useState<SubordinateRow[]>([]);
  const [subsLoading, setSubsLoading] = useState(true);

  // Initialize draft from user (passed in via auth context). Re-sync
  // when the user object changes (e.g. after refresh()).
  useEffect(() => {
    if (!user) return;
    setDraft(
      isAllowed(user.tier2_contribution_pct)
        ? user.tier2_contribution_pct
        : nearestAllowed(user.tier2_contribution_pct),
    );
  }, [user]);

  // Fetch the caller's subordinates so they can see WHO this setting
  // applies to. Refreshed on first mount only — the list changes only
  // when admin reassigns hierarchy, which is rare.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const rows = await kolApi.listMySubordinates();
        if (!cancelled) setSubs(rows);
      } catch (e) {
        // Non-fatal: panel still shows the bucket selector even if
        // the subordinate list fails to load.
        console.warn("listMySubordinates failed:", e);
      } finally {
        if (!cancelled) setSubsLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const dirty = user !== null && draft !== user.tier2_contribution_pct;

  const handleSave = useCallback(async () => {
    if (!dirty) return;
    setSaving(true);
    setError(null);
    try {
      await kolApi.updateMyTier2Contribution({ tier2_contribution_pct: draft });
      // Pull /me again so the auth context reflects the new value
      // (the team-nav visibility could in theory change too, though
      // it's based on has_subordinates not on the pct).
      await refresh();
      setSavedFlash(true);
      setTimeout(() => setSavedFlash(false), 2000);
    } catch (e) {
      setError(e instanceof Error ? e.message : "保存失败");
    } finally {
      setSaving(false);
    }
  }, [draft, dirty, refresh]);

  if (!user) return null;

  return (
    <div className="flex flex-col gap-6 max-w-2xl">
      <div>
        <h2 className="text-lg font-semibold">下级贡献度</h2>
        <p className="text-xs text-muted-foreground">
          配置我的二级用户(下级)采集到的词中,有多少比例自动汇入到我的账号池。
          管理员的全局贡献度优先生效,这里设置的是「管理员拿走之后剩下的词」中,
          下级再贡献给我的比例。
        </p>
      </div>

      {error && (
        <ErrorBanner>{error}</ErrorBanner>
      )}

      <section className="rounded-lg border p-4 flex flex-col gap-4">
        <header className="flex flex-col gap-1">
          <h3 className="text-base font-semibold">我的二级贡献度</h3>
          <p className="text-[11px] text-muted-foreground leading-relaxed">
            两层贡献按顺序执行:每个词先过管理员累加器(全局设置),
            没被管理员拿走的词再过这里的累加器。失败的词不计入,
            管理员或我的池子如果满了会自动回退到其他可用池,不会丢词。
          </p>
        </header>

        <fieldset className="flex flex-col gap-2" disabled={saving}>
          <legend className="text-xs font-medium text-muted-foreground mb-1">
            选择档位
          </legend>
          <div className="grid grid-cols-5 gap-2">
            {TIER2_OPTIONS.map((opt) => {
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
            {TIER2_OPTIONS.find((o) => o.pct === draft)?.cadence}
          </div>
        </fieldset>

        <footer className="flex items-center gap-3 pt-2 border-t">
          <Button onClick={handleSave} disabled={!dirty || saving}>
            {saving ? "保存中..." : dirty ? "保存修改" : "已保存"}
          </Button>
          {savedFlash && <span className="text-xs text-success">✓ 已应用</span>}
          {!dirty && (
            <span className="text-xs text-muted-foreground">
              当前生效值: {user.tier2_contribution_pct}%
            </span>
          )}
        </footer>
      </section>

      <section className="rounded-lg border p-4 flex flex-col gap-3">
        <header className="flex items-baseline justify-between gap-2">
          <h3 className="text-base font-semibold">我的下级</h3>
          <span className="text-xs text-muted-foreground">
            共 {subs.length} 个 ·{" "}
            {subs.filter((s) => s.is_active).length} 启用
          </span>
        </header>
        {subsLoading ? (
          <p className="text-xs text-muted-foreground">加载中...</p>
        ) : subs.length === 0 ? (
          // Defensive — shouldn't normally reach this since the panel
          // is gated on has_subordinates. Guard against the race where
          // admin removes the last sub while the page is open.
          <p className="text-xs text-muted-foreground">
            当前没有下级。如果你刚被取消下级关系,请刷新页面。
          </p>
        ) : (
          <ul className="flex flex-col divide-y">
            {subs.map((s) => (
              <li
                key={s.id}
                className="flex items-center justify-between gap-2 py-2 text-sm"
              >
                <div className="flex items-center gap-2 min-w-0">
                  <span className="font-medium truncate">{s.username}</span>
                  {s.email && (
                    <span
                      className="text-xs text-muted-foreground truncate"
                      title={s.email}
                    >
                      · {s.email}
                    </span>
                  )}
                </div>
                <Badge
                  variant={s.is_active ? "outline" : "destructive"}
                  className="text-[10px] shrink-0"
                >
                  {s.is_active ? "启用" : "停用"}
                </Badge>
              </li>
            ))}
          </ul>
        )}
        <p className="text-[11px] text-muted-foreground">
          下级关系由管理员维护。如需新增/删除/调换,请联系管理员。
        </p>
      </section>
    </div>
  );
}
