"use client";

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
        <div className="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      )}

      <div className="rounded-md border overflow-x-auto">
        <Table className="min-w-[900px]">
          <TableHeader>
            <TableRow>
              <TableHead className="w-28">月份</TableHead>
              <TableHead>账号 / 所属</TableHead>
              <TableHead className="min-w-[260px]">标题</TableHead>
              <TableHead>收件人</TableHead>
              <TableHead className="w-32">发送时间</TableHead>
              <TableHead className="w-20 text-right">操作</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading && rows.length === 0 ? (
              <TableRow>
                <TableCell colSpan={6} className="text-center text-muted-foreground">
                  加载中...
                </TableCell>
              </TableRow>
            ) : rows.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={6}
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
