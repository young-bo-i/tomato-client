"use client";
import { ErrorBanner } from "./shared/error-banner";

import { useCallback, useEffect, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { kolApi } from "../api/client";
import type { QimaoNoticeRow } from "../types";

/// Admin panel: 七猫达人 monthly income notice history.
///
/// The cron job polls every qimao profile's message feed 3× a day on
/// days 10–20, finds notices titled "X月KOC七猫免费小说收益明细",
/// and emails them as HTML. This panel surfaces the resulting ledger
/// (`qimao_income_notice` table) so admin can confirm what was sent
/// and replay anything that failed.
///
/// Click a row → modal showing the upstream's HTML content rendered
/// in a sandboxed iframe. iframe srcdoc keeps the inline styles from
/// affecting the rest of the page.
export function KolQimaoNoticePanel() {
  const [rows, setRows] = useState<QimaoNoticeRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [viewing, setViewing] = useState<QimaoNoticeRow | null>(null);

  const refresh = useCallback(async () => {
    setError(null);
    try {
      const list = await kolApi.listQimaoNotices();
      setRows(list);
    } catch (e) {
      setError(e instanceof Error ? e.message : "加载失败");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    // Quiet refresh every 60s while visible. Cron fires 3×/day on
    // 10–20, so faster polling is wasted — but a soft auto-refresh
    // keeps the page useful when the operator leaves it open.
    const t = setInterval(() => {
      if (document.visibilityState === "visible") void refresh();
    }, 60000);
    return () => clearInterval(t);
  }, [refresh]);

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <h2 className="text-lg font-semibold">七猫收益通知</h2>
          <p className="text-xs text-muted-foreground">
            每月 10–20 日,9 点 / 13 点 / 21 点各扫一次。
            匹配到「X月KOC七猫免费小说收益明细」标题就发邮件给所有者。
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={() => void refresh()}>
          刷新
        </Button>
      </div>

      {error && (
        <ErrorBanner>{error}</ErrorBanner>
      )}

      <GrandTotalBanner rows={rows} />

      <div className="rounded-md border overflow-x-auto">
        <Table className="min-w-[1000px]">
          <TableHeader>
            <TableRow>
              <TableHead className="w-28">月份</TableHead>
              <TableHead>账号 / 所属</TableHead>
              <TableHead className="min-w-[200px]">标题</TableHead>
              <TableHead className="w-28 text-right">总收益</TableHead>
              <TableHead className="w-44">明细</TableHead>
              <TableHead>收件人</TableHead>
              <TableHead className="w-32">发送时间</TableHead>
              <TableHead className="w-20 text-right">操作</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading && rows.length === 0 ? (
              <TableRow>
                <TableCell colSpan={8} className="text-center text-muted-foreground">
                  加载中...
                </TableCell>
              </TableRow>
            ) : rows.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={8}
                  className="text-center text-muted-foreground py-6"
                >
                  暂无通知记录 — 等待下一次月度收益结算
                </TableCell>
              </TableRow>
            ) : (
              rows.map((r) => (
                <TableRow key={`${r.profile_id}-${r.message_id}`}>
                  <TableCell className="font-mono text-xs align-top">
                    {r.notice_date ?? "—"}
                  </TableCell>
                  <TableCell className="align-top">
                    <div className="flex flex-col gap-0.5">
                      <span className="text-xs font-medium truncate max-w-[180px]">
                        {r.profile_name}
                      </span>
                      <span className="text-[10px] text-muted-foreground">
                        @{r.owner_username}
                      </span>
                    </div>
                  </TableCell>
                  <TableCell className="align-top">
                    <span className="text-sm">{r.title}</span>
                  </TableCell>
                  <TableCell className="align-top text-right font-mono">
                    <span
                      className={
                        r.total_income_cents && r.total_income_cents > 0
                          ? "text-success font-semibold"
                          : "text-muted-foreground"
                      }
                    >
                      {fmtYuan(r.total_income_cents)}
                    </span>
                  </TableCell>
                  <TableCell className="align-top">
                    <BreakdownChips row={r} />
                  </TableCell>
                  <TableCell className="align-top">
                    <span className="text-xs text-muted-foreground">
                      {r.recipient_email ?? "—"}
                    </span>
                  </TableCell>
                  <TableCell className="align-top">
                    <SendStatusBadge row={r} />
                  </TableCell>
                  <TableCell className="text-right align-top">
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => setViewing(r)}
                    >
                      查看
                    </Button>
                  </TableCell>
                </TableRow>
              ))
            )}
          </TableBody>
        </Table>
      </div>

      <NoticeDetailDialog
        notice={viewing}
        onOpenChange={(open) => {
          if (!open) setViewing(null);
        }}
      />
    </div>
  );
}

function SendStatusBadge({ row }: { row: QimaoNoticeRow }) {
  if (row.emailed_at) {
    return (
      <div className="flex flex-col gap-0.5">
        <Badge variant="outline" className="text-[10px] w-fit">
          ✓ 已发送
        </Badge>
        <span className="text-[10px] text-muted-foreground font-mono">
          {fmtClock(row.emailed_at)}
        </span>
      </div>
    );
  }
  if (row.send_error) {
    return (
      <Badge
        variant="destructive"
        className="text-[10px]"
        title={row.send_error}
      >
        ✗ 失败
      </Badge>
    );
  }
  return (
    <Badge variant="secondary" className="text-[10px]">
      —
    </Badge>
  );
}

/// Render the upstream HTML in an iframe `srcdoc` so the inline
/// styles can't leak into the parent page (the upstream uses
/// inline `font-size:20px` etc that would clash with the dialog
/// chrome). Sized to fit the dialog body.
function NoticeDetailDialog({
  notice,
  onOpenChange,
}: {
  notice: QimaoNoticeRow | null;
  onOpenChange: (open: boolean) => void;
}) {
  return (
    <Dialog open={notice !== null} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>{notice?.title ?? ""}</DialogTitle>
        </DialogHeader>
        {notice && (
          <div className="flex flex-col gap-3">
            <div className="grid grid-cols-4 gap-2">
              <AmountCard label="总收益" cents={notice.total_income_cents} highlight />
              <AmountCard label="拉新" cents={notice.new_user_income_cents} />
              <AmountCard label="拉活" cents={notice.active_income_cents} />
              <AmountCard label="拉新激励" cents={notice.new_user_bonus_cents} />
            </div>
            <div className="grid grid-cols-2 gap-2 text-xs">
              <Stat label="账号" value={notice.profile_name} />
              <Stat label="所属用户" value={`@${notice.owner_username}`} />
              <Stat label="月份" value={notice.notice_date ?? "—"} />
              <Stat
                label="收件人"
                value={notice.recipient_email ?? "—"}
              />
              <Stat
                label="发送时间"
                value={
                  notice.emailed_at ? fmtClock(notice.emailed_at) : "未发送"
                }
              />
              <Stat
                label="状态"
                value={
                  notice.emailed_at
                    ? "✓ 已发送"
                    : notice.send_error
                      ? "✗ 失败"
                      : "—"
                }
              />
            </div>
            {notice.send_error && (
              <div className="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-xs text-destructive">
                <strong>SMTP 错误:</strong> {notice.send_error}
              </div>
            )}
            <div>
              <p className="text-xs text-muted-foreground mb-1">
                通知正文(平台原始 HTML):
              </p>
              <iframe
                title={notice.title}
                srcDoc={notice.content_html}
                sandbox=""
                className="w-full h-72 border rounded-md bg-card"
              />
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-md border bg-card p-2 flex flex-col">
      <span className="text-[10px] text-muted-foreground">{label}</span>
      <span className="text-sm font-mono truncate" title={value}>
        {value}
      </span>
    </div>
  );
}

function fmtClock(iso: string): string {
  const d = new Date(iso);
  return `${d.getMonth() + 1}/${d.getDate()} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

/// Format cents as ¥XX.XX. NULL → "—". Used both in table cells and
/// the detail dialog cards.
function fmtYuan(cents: number | null): string {
  if (cents === null || cents === undefined) return "—";
  return `¥${(cents / 100).toFixed(2)}`;
}

/// Compact pills showing the 3 sub-breakdown amounts when present.
/// Hidden when all three are NULL (parser missed everything).
function BreakdownChips({ row }: { row: QimaoNoticeRow }) {
  const items: Array<{ label: string; cents: number | null }> = [
    { label: "拉新", cents: row.new_user_income_cents },
    { label: "拉活", cents: row.active_income_cents },
    { label: "激励", cents: row.new_user_bonus_cents },
  ];
  const anyPresent = items.some((i) => i.cents !== null);
  if (!anyPresent) {
    return <span className="text-[10px] text-muted-foreground">—</span>;
  }
  return (
    <div className="flex flex-wrap gap-1">
      {items.map((i) => (
        <span
          key={i.label}
          className="inline-flex items-center gap-1 rounded-md border bg-card px-1.5 py-0.5 text-[10px] font-mono"
        >
          <span className="text-muted-foreground">{i.label}</span>
          <span
            className={
              i.cents && i.cents > 0 ? "font-semibold" : "text-muted-foreground"
            }
          >
            {fmtYuan(i.cents)}
          </span>
        </span>
      ))}
    </div>
  );
}

/// Top-of-panel banner summarizing the total income across all
/// currently-loaded rows. Useful for KOLs to eyeball cumulative income
/// without doing the math themselves.
function GrandTotalBanner({ rows }: { rows: QimaoNoticeRow[] }) {
  const grandCents = rows.reduce(
    (acc, r) => acc + (r.total_income_cents ?? 0),
    0,
  );
  const countWithAmount = rows.filter(
    (r) => r.total_income_cents !== null && r.total_income_cents > 0,
  ).length;
  if (grandCents <= 0) return null;
  return (
    <div className="rounded-md border bg-success/10 border-success/40 px-4 py-3">
      <div className="flex items-baseline gap-2">
        <span className="text-xs text-muted-foreground">累计收益</span>
        <span className="text-2xl font-bold font-mono text-success">
          {fmtYuan(grandCents)}
        </span>
        <span className="text-xs text-muted-foreground">
          · {countWithAmount} 条已解析
        </span>
      </div>
    </div>
  );
}

function AmountCard({
  label,
  cents,
  highlight = false,
}: {
  label: string;
  cents: number | null;
  highlight?: boolean;
}) {
  return (
    <div
      className={
        highlight
          ? "rounded-md border-2 border-success/60 bg-success/10 p-2 flex flex-col"
          : "rounded-md border bg-card p-2 flex flex-col"
      }
    >
      <span className="text-[10px] text-muted-foreground">{label}</span>
      <span
        className={
          highlight
            ? "text-base font-semibold font-mono text-success"
            : "text-sm font-mono"
        }
      >
        {fmtYuan(cents)}
      </span>
    </div>
  );
}
