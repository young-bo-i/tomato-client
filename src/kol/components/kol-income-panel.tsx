"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { kolApi } from "../api/client";
import type { IncomeOverview, IncomeRow } from "../types";

/// Admin panel:番茄达人收益看板. Shows the latest snapshot per
/// tomato profile, summed up at the top, refreshed every 30s while
/// visible (the underlying poller cadence is 10 min so faster polling
/// here just keeps the UI fresh against fetched_at — not against the
/// upstream).
///
/// All amounts on the wire are 分 (cents). Display divides by 100.
export function KolIncomePanel() {
  const [rows, setRows] = useState<IncomeRow[]>([]);
  const [overview, setOverview] = useState<IncomeOverview | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());

  const refresh = useCallback(async () => {
    setError(null);
    try {
      const [list, ov] = await Promise.all([
        kolApi.listIncome(),
        kolApi.getIncomeOverview(),
      ]);
      setRows(list);
      setOverview(ov);
    } catch (e) {
      setError(e instanceof Error ? e.message : "加载失败");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    let timer: ReturnType<typeof setInterval> | null = null;
    const start = () => {
      if (timer !== null) return;
      void refresh();
      timer = setInterval(() => void refresh(), 30000);
    };
    const stop = () => {
      if (timer !== null) {
        clearInterval(timer);
        timer = null;
      }
    };
    const onVisibility = () => {
      if (document.visibilityState === "visible") start();
      else stop();
    };
    if (document.visibilityState === "visible") start();
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      stop();
    };
  }, [refresh]);

  // Tick once a second so the "last fetched X seconds ago" age stays
  // accurate without a full refresh.
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);

  // Show a "🆙 +¥XX" highlight on rows whose last_diff_at is within
  // the last 30 minutes — these are the actively-earning accounts the
  // operator probably wants to see first.
  const recentDiffWindow = 30 * 60 * 1000;
  const recentRows = useMemo(
    () =>
      rows.filter(
        (r) =>
          r.last_diff > 0 &&
          r.last_diff_at !== null &&
          now - new Date(r.last_diff_at).getTime() < recentDiffWindow,
      ),
    [rows, now, recentDiffWindow],
  );

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <h2 className="text-lg font-semibold">番茄收益看板</h2>
          <p className="text-xs text-muted-foreground">
            每 10 分钟拉取一次,带 2 分钟时间窗口防抖。所有金额单位为元。
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={() => void refresh()}>
          刷新
        </Button>
      </div>

      {error && (
        <div className="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      )}

      {/* Aggregated overview header */}
      {overview && (
        <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-6 gap-2 text-xs">
          <Stat label="账号数" value={overview.account_count.toString()} />
          <Stat
            label="总收益"
            value={fmtYuan(overview.total_income)}
            tone="good"
          />
          <Stat label="常规" value={fmtYuan(overview.regular_income)} />
          <Stat label="激励" value={fmtYuan(overview.bonus_income)} />
          <Stat label="本月" value={fmtYuan(overview.current_month_income)} />
          <Stat label="本周" value={fmtYuan(overview.current_week_income)} />
        </div>
      )}

      {overview?.last_fetched_at && (
        <p className="text-[11px] text-muted-foreground">
          最近拉取:{fmtClock(overview.last_fetched_at)} ·{" "}
          {fmtAge(now - new Date(overview.last_fetched_at).getTime())}前
        </p>
      )}

      {recentRows.length > 0 && (
        <div className="rounded-md border border-success/40 bg-success/5 px-3 py-2 text-xs">
          <span className="font-semibold text-success">
            ✨ 最近 30 分钟内有 {recentRows.length} 个账号收益增长
          </span>
        </div>
      )}

      <div className="rounded-md border overflow-x-auto">
        <Table className="min-w-[900px]">
          <TableHeader>
            <TableRow>
              <TableHead className="min-w-[140px]">账号</TableHead>
              <TableHead className="w-28">所属用户</TableHead>
              <TableHead className="w-28 text-right">总收益</TableHead>
              <TableHead className="w-24 text-right">常规</TableHead>
              <TableHead className="w-24 text-right">激励</TableHead>
              <TableHead className="w-24 text-right">本月</TableHead>
              <TableHead className="w-24 text-right">本周</TableHead>
              <TableHead className="w-28">最近变动</TableHead>
              <TableHead className="w-28">邮件通知</TableHead>
              <TableHead className="w-32">最后更新</TableHead>
              <TableHead className="w-24">轮询</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading && rows.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={11}
                  className="text-center text-muted-foreground"
                >
                  加载中...
                </TableCell>
              </TableRow>
            ) : rows.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={11}
                  className="text-center text-muted-foreground py-6"
                >
                  暂无收益数据 — 等待下一次轮询(每 10 分钟)
                </TableCell>
              </TableRow>
            ) : (
              rows.map((r) => {
                const diffRecent =
                  r.last_diff > 0 &&
                  r.last_diff_at !== null &&
                  now - new Date(r.last_diff_at).getTime() < recentDiffWindow;
                const fetchedAge = now - new Date(r.fetched_at).getTime();
                const stale = fetchedAge > 15 * 60 * 1000;
                return (
                  <TableRow
                    key={r.profile_id}
                    className={diffRecent ? "bg-success/5" : undefined}
                  >
                    <TableCell className="align-top">
                      <div className="flex flex-col gap-0.5">
                        <span className="font-medium truncate max-w-[180px]">
                          {r.profile_name}
                        </span>
                        <span className="text-[10px] text-muted-foreground font-mono">
                          {r.profile_id.slice(0, 8)}
                        </span>
                      </div>
                    </TableCell>
                    <TableCell className="align-top">
                      <div className="flex flex-col gap-0.5">
                        <span className="text-xs">{r.owner_username}</span>
                        {r.owner_role === "admin" && (
                          <Badge variant="default" className="text-[10px] w-fit">
                            管理员
                          </Badge>
                        )}
                      </div>
                    </TableCell>
                    <TableCell className="text-right font-mono font-semibold align-top">
                      {fmtYuan(r.total_income)}
                    </TableCell>
                    <TableCell className="text-right font-mono text-xs text-muted-foreground align-top">
                      {fmtYuan(r.regular_income)}
                    </TableCell>
                    <TableCell className="text-right font-mono text-xs text-muted-foreground align-top">
                      {fmtYuan(r.bonus_income)}
                    </TableCell>
                    <TableCell className="text-right font-mono text-xs align-top">
                      {fmtYuan(r.current_month_income)}
                    </TableCell>
                    <TableCell className="text-right font-mono text-xs align-top">
                      {fmtYuan(r.current_week_income)}
                    </TableCell>
                    <TableCell className="align-top">
                      {r.last_diff > 0 && r.last_diff_at ? (
                        <div className="flex flex-col gap-0.5">
                          <span
                            className={`text-xs font-mono font-semibold ${
                              diffRecent ? "text-success" : "text-muted-foreground"
                            }`}
                          >
                            +{fmtYuan(r.last_diff)}
                          </span>
                          <span className="text-[10px] text-muted-foreground">
                            {fmtClock(r.last_diff_at)}
                          </span>
                        </div>
                      ) : (
                        <span className="text-xs text-muted-foreground">—</span>
                      )}
                    </TableCell>
                    <TableCell className="align-top">
                      <EmailStatusBadge row={r} />
                    </TableCell>
                    <TableCell className="text-xs text-muted-foreground align-top">
                      {r.latest_update_time
                        ? fmtClock(r.latest_update_time)
                        : "—"}
                    </TableCell>
                    <TableCell className="align-top">
                      <span
                        className={`text-[10px] font-mono ${
                          stale ? "text-warning" : "text-muted-foreground"
                        }`}
                        title={fmtClock(r.fetched_at)}
                      >
                        {fmtAge(fetchedAge)}前
                      </span>
                    </TableCell>
                  </TableRow>
                );
              })
            )}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}

function EmailStatusBadge({ row }: { row: IncomeRow }) {
  // 4 states:
  //   * No diff yet → "—"
  //   * Diff but no email attempt yet (first round just persisted,
  //     email step skipped due to no SMTP / etc.) → 待发送
  //   * Email succeeded after the diff → ✓ 已发送 + timestamp
  //   * Email attempted but failed (last_email_error set, last_emailed_at
  //     either NULL or older than last_diff_at) → ✗ 失败 + tooltip
  if (!row.last_diff_at) {
    return <span className="text-xs text-muted-foreground">—</span>;
  }
  const diffMs = new Date(row.last_diff_at).getTime();
  const emailedMs = row.last_emailed_at
    ? new Date(row.last_emailed_at).getTime()
    : 0;
  const sent = emailedMs >= diffMs && row.last_emailed_at !== null;

  if (sent) {
    return (
      <div className="flex flex-col gap-0.5">
        <span className="text-[10px] inline-flex items-center gap-1 rounded-md border border-success/40 bg-success/10 text-success px-1.5 py-0.5 w-fit">
          ✓ 已发送
        </span>
        <span className="text-[10px] text-muted-foreground font-mono">
          {row.last_emailed_at ? fmtClock(row.last_emailed_at) : "—"}
        </span>
      </div>
    );
  }
  if (row.last_email_error) {
    return (
      <span
        className="text-[10px] inline-flex items-center gap-1 rounded-md border border-destructive/50 bg-destructive/10 text-destructive px-1.5 py-0.5 w-fit"
        title={row.last_email_error}
      >
        ✗ 失败
      </span>
    );
  }
  return (
    <span className="text-[10px] inline-flex items-center gap-1 rounded-md border bg-muted text-muted-foreground px-1.5 py-0.5 w-fit">
      待发送
    </span>
  );
}

function Stat({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone?: "good";
}) {
  const toneCls =
    tone === "good" ? "border-success/40 bg-success/10" : "bg-card";
  return (
    <div className={`rounded-md border ${toneCls} p-2 flex flex-col`}>
      <span className="text-[10px] text-muted-foreground">{label}</span>
      <span className="text-sm font-mono font-semibold tabular-nums">
        {value}
      </span>
    </div>
  );
}

/** 分 → 元,2 decimals. */
function fmtYuan(cents: number): string {
  return `¥${(cents / 100).toFixed(2)}`;
}

function fmtClock(iso: string): string {
  const d = new Date(iso);
  return `${d.getMonth() + 1}/${d.getDate()} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

function fmtAge(ms: number): string {
  if (ms < 60_000) return `${Math.floor(ms / 1000)}s`;
  if (ms < 3_600_000) return `${Math.floor(ms / 60_000)}m`;
  if (ms < 86_400_000) return `${Math.floor(ms / 3_600_000)}h`;
  return `${Math.floor(ms / 86_400_000)}d`;
}
