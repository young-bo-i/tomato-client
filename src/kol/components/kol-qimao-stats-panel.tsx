"use client";
import { ErrorBanner } from "./shared/error-banner";

import { useCallback, useEffect, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useVisibilityInterval } from "../hooks/use-visibility-interval";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { kolApi } from "../api/client";
import type { QimaoStatsAccount, QimaoStatsOverview } from "../types";
import { StatCard } from "./shared/stat-card";
import { formatRelative } from "../lib/format";

const REFRESH_INTERVAL_MS = 30_000;

export function KolQimaoStatsPanel() {
  const [overview, setOverview] = useState<QimaoStatsOverview | null>(null);
  const [accounts, setAccounts] = useState<QimaoStatsAccount[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [lastUpdated, setLastUpdated] = useState<Date | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      const [ov, acc] = await Promise.all([
        kolApi.getQimaoStatsOverview(),
        kolApi.getQimaoStatsAccounts(),
      ]);
      setOverview(ov);
      setAccounts(acc);
      setLastUpdated(new Date());
    } catch (e) {
      setError(e instanceof Error ? e.message : "加载失败");
    } finally {
      setLoading(false);
    }
  }, []);

  // Initial fetch on mount.
  useEffect(() => { void load(); }, [load]);
  // Poll every 30 s; pauses automatically when the window is hidden.
  useVisibilityInterval(load, REFRESH_INTERVAL_MS);

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div className="min-w-0">
          <h2 className="text-lg font-semibold">七猫达人数据看板</h2>
          <p className="text-xs text-muted-foreground">
            每 30 秒自动刷新。账号"无 token"代表 server 还没成功 signin (12
            小时自动重试,也可在七猫书籍 tab 手动触发)。
            {lastUpdated && (
              <> · 最近更新: {lastUpdated.toLocaleTimeString()}</>
            )}
          </p>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={() => void load()}
          disabled={loading}
          className="shrink-0"
        >
          手动刷新
        </Button>
      </div>

      {error && (
        <ErrorBanner>{error}</ErrorBanner>
      )}

      {/* === Overview tiles ===
          One extra tile vs tomato: "等待 alias_id" — qimao's add_keywords
          doesn't return the platform-side id, the backfill worker has to
          poll keyword_page to find it. This count makes the gap visible. */}
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-4 lg:grid-cols-8">
        <StatCard label="总词数" value={overview?.total ?? 0} tone="neutral" />
        <StatCard
          label="申请待处理"
          value={overview?.submit_pending ?? 0}
          tone="warning"
        />
        <StatCard
          label="申请成功"
          value={overview?.submit_done ?? 0}
          tone="success"
        />
        <StatCard
          label="申请失败"
          value={overview?.submit_failed ?? 0}
          tone="destructive"
        />
        <StatCard
          label="等待 alias_id"
          value={overview?.awaiting_alias_id ?? 0}
          tone="muted"
        />
        <StatCard
          label="回填待处理"
          value={overview?.backfill_pending ?? 0}
          tone="warning"
        />
        <StatCard
          label="回填成功"
          value={overview?.backfill_done ?? 0}
          tone="success"
        />
        <StatCard
          label="回填失败"
          value={overview?.backfill_failed ?? 0}
          tone="destructive"
        />
      </div>

      {/* === Per-account table === */}
      <div className="rounded-md border overflow-x-auto">
        <Table className="min-w-[820px]">
          <TableHeader>
            <TableRow>
              <TableHead>账号</TableHead>
              <TableHead className="w-32">手机号</TableHead>
              <TableHead className="w-24">Token 状态</TableHead>
              <TableHead className="w-32">最近刷新</TableHead>
              <TableHead className="w-24 text-right">申请成功</TableHead>
              <TableHead className="w-24 text-right">申请失败</TableHead>
              <TableHead className="w-24 text-right">回填成功</TableHead>
              <TableHead className="w-24 text-right">回填失败</TableHead>
              <TableHead>最近报错</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading && accounts.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={9}
                  className="text-center text-muted-foreground"
                >
                  加载中...
                </TableCell>
              </TableRow>
            ) : accounts.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={9}
                  className="text-center text-muted-foreground"
                >
                  暂无七猫 profile — 在新建 profile 时选"七猫达人"并填账号密码
                </TableCell>
              </TableRow>
            ) : (
              accounts.map((a) => (
                <TableRow key={a.profile_id}>
                  <TableCell className="font-medium">
                    {a.profile_name}
                  </TableCell>
                  <TableCell className="text-xs text-muted-foreground">
                    {a.qimao_identifier ?? "—"}
                  </TableCell>
                  <TableCell>
                    {a.has_token ? (
                      <Badge
                        variant="outline"
                        className="border-success/50 bg-success/10 text-success"
                      >
                        在线
                      </Badge>
                    ) : (
                      <Badge variant="destructive">无 token</Badge>
                    )}
                  </TableCell>
                  <TableCell className="text-xs text-muted-foreground">
                    {formatRelative(a.qimao_token_refreshed_at)}
                  </TableCell>
                  <TableCell className="text-right font-mono tabular-nums">
                    {a.submit_done.toLocaleString()}
                  </TableCell>
                  <TableCell className="text-right font-mono tabular-nums text-destructive">
                    {a.submit_failed.toLocaleString()}
                  </TableCell>
                  <TableCell className="text-right font-mono tabular-nums">
                    {a.backfill_done.toLocaleString()}
                  </TableCell>
                  <TableCell className="text-right font-mono tabular-nums text-destructive">
                    {a.backfill_failed.toLocaleString()}
                  </TableCell>
                  <TableCell
                    className="max-w-[260px] truncate text-xs text-muted-foreground"
                    title={a.qimao_token_last_error ?? undefined}
                  >
                    {a.qimao_token_last_error ?? "—"}
                  </TableCell>
                </TableRow>
              ))
            )}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}
