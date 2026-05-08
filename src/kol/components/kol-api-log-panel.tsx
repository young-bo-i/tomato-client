"use client";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useCallback, useEffect, useRef, useState } from "react";
import { kolApi } from "../api/client";
import type { ApiLogQuery, ApiLogRow, PagedApiLog } from "../types";

const PAGE_SIZE = 20;

const SERVICE_LABELS: Record<string, string> = {
  fanqie_promotion: "番茄达人",
  qimao_promotion: "七猫达人",
};

const ENDPOINT_LABELS: Record<string, string> = {
  "promotion/plan/create": "别名创建",
  "promotion/post/create": "链接回填",
  "promotion/plan/list": "别名查询",
  "platform/ranking/rank_list": "书籍榜单",
  "promotion/keyword_precheck": "关键词预检",
  "promotion/add_keywords": "关键词提交",
  "promotion/keyword_page": "关键词查询",
  "promotion/add_keyword_links": "关键词链接",
  "data/book/index": "书籍列表",
  "user/signin": "账号登录",
};

function fmt(date: string | null): string {
  if (!date) return "—";
  return new Date(date).toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function triggerCsvDownload(csv: string, filename: string) {
  const blob = new Blob(["﻿" + csv], { type: "text/csv;charset=utf-8;" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.style.display = "none";
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 2000);
}

function formatRowForCopy(row: ApiLogRow): string {
  return JSON.stringify(
    {
      id: row.id,
      created_at: row.created_at,
      service: row.service,
      endpoint: row.endpoint,
      http_status: row.http_status,
      parsed_ok: row.parsed_ok,
      acknowledged: row.acknowledged,
      parse_error: row.parse_error,
      request_summary: row.request_summary,
      raw_response: row.raw_response,
    },
    null,
    2,
  );
}

// ── 过滤条状态 ─────────────────────────────────────────────────────────────

interface Filters {
  service: string;
  endpoint: string;
  parsed_ok: string;  // "all" | "true" | "false"
  acknowledged: string; // "all" | "true" | "false"
}

const DEFAULT_FILTERS: Filters = {
  service: "all",
  endpoint: "all",
  parsed_ok: "false",   // 默认只看失败
  acknowledged: "false", // 默认只看未标记
};

function filtersToQuery(f: Filters, page: number, pageSize: number): ApiLogQuery {
  const q: ApiLogQuery = { page, page_size: pageSize };
  if (f.service !== "all") q.service = f.service;
  if (f.endpoint !== "all") q.endpoint = f.endpoint;
  if (f.parsed_ok !== "all") q.parsed_ok = f.parsed_ok === "true";
  if (f.acknowledged !== "all") q.acknowledged = f.acknowledged === "true";
  return q;
}

// ── 主组件 ────────────────────────────────────────────────────────────────

export function KolApiLogPanel() {
  const [data, setData] = useState<PagedApiLog | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const [filters, setFilters] = useState<Filters>(DEFAULT_FILTERS);
  const [page, setPage] = useState(1);

  // 多选状态
  const [selected, setSelected] = useState<Set<number>>(new Set());

  // 详情弹窗
  const [detail, setDetail] = useState<ApiLogRow | null>(null);

  // 操作状态
  const [acting, setActing] = useState(false);
  const [exporting, setExporting] = useState(false);
  // 复制反馈：记录最近复制的行 id，短暂显示"✓"后清除
  const [copiedId, setCopiedId] = useState<number | null>(null);
  const copyTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Stable callback — always called with explicit (f, p) args, never reads
  // from its own closure. This prevents useCallback from being recreated on
  // every filters/page change, eliminating the double-fetch that occurred when
  // applyFilter called load() directly AND the useEffect also fired.
  const load = useCallback(async (f: Filters, p: number) => {
    setLoading(true);
    setError(null);
    try {
      const result = await kolApi.listApiLog(filtersToQuery(f, p, PAGE_SIZE));
      setData(result);
      setSelected(new Set());
    } catch (e) {
      setError(typeof e === "string" ? e : (e as Error).message);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load(filters, page);
  }, [filters, page, load]);

  // Clean up the copy-feedback timer on unmount to prevent setState on an
  // already-unmounted component.
  useEffect(() => {
    return () => {
      if (copyTimerRef.current) clearTimeout(copyTimerRef.current);
    };
  }, []);

  // ── 过滤变化时重置到第一页 ───────────────────────────────────────────────
  // Only setState here. The useEffect([filters, page, load]) drives the
  // actual fetch — calling load() here directly as well caused two identical
  // requests on every filter change.
  function applyFilter<K extends keyof Filters>(key: K, val: Filters[K]) {
    setFilters({ ...filters, [key]: val });
    setPage(1);
    setSelected(new Set());
    setLoading(true); // optimistic: show spinner before useEffect fires
  }

  // ── 全选当页 ─────────────────────────────────────────────────────────────
  function toggleAll(checked: boolean) {
    if (!data) return;
    if (checked) {
      setSelected(new Set(data.rows.map((r) => r.id)));
    } else {
      setSelected(new Set());
    }
  }

  function toggleOne(id: number) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  // ── 批量标记 ─────────────────────────────────────────────────────────────
  async function handleMark(acknowledged: boolean) {
    if (selected.size === 0) return;
    setActing(true);
    try {
      await kolApi.markApiLog({ ids: [...selected], acknowledged });
      void load(filters, page);
    } catch (e) {
      alert((e as Error).message ?? "操作失败");
    } finally {
      setActing(false);
    }
  }

  // ── 批量删除 ─────────────────────────────────────────────────────────────
  async function handleDelete() {
    if (selected.size === 0) return;
    if (!confirm(`确认删除选中的 ${selected.size} 条记录？此操作不可撤销。`)) return;
    setActing(true);
    try {
      await kolApi.deleteApiLog({ ids: [...selected] });
      void load(filters, page);
    } catch (e) {
      alert((e as Error).message ?? "删除失败");
    } finally {
      setActing(false);
    }
  }

  // ── 复制单行 ──────────────────────────────────────────────────────────────
  function handleCopyRow(row: ApiLogRow) {
    void navigator.clipboard.writeText(formatRowForCopy(row)).then(() => {
      setCopiedId(row.id);
      if (copyTimerRef.current) clearTimeout(copyTimerRef.current);
      copyTimerRef.current = setTimeout(() => setCopiedId(null), 1500);
    });
  }

  // ── 导出 ─────────────────────────────────────────────────────────────────
  async function handleExport() {
    setExporting(true);
    try {
      const result = await kolApi.exportApiLog(filtersToQuery(filters, 1, 5000));
      const ts = new Date().toISOString().slice(0, 10);
      triggerCsvDownload(result.csv, `api_log_${ts}.csv`);
    } catch (e) {
      alert((e as Error).message ?? "导出失败");
    } finally {
      setExporting(false);
    }
  }

  // ── 分页辅助 ─────────────────────────────────────────────────────────────
  const totalPages = data ? Math.ceil(data.total / PAGE_SIZE) : 1;
  const allPageSelected =
    data != null &&
    data.rows.length > 0 &&
    data.rows.every((r) => selected.has(r.id));

  // ── render ────────────────────────────────────────────────────────────────
  return (
    <div className="space-y-3">
      {/* 标题行 */}
      <div className="flex items-center justify-between gap-2 flex-wrap">
        <h3 className="text-sm font-semibold shrink-0">外部接口请求日志</h3>
        <div className="flex items-center gap-2 flex-wrap">
          <Button
            variant="outline"
            size="sm"
            onClick={() => void load(filters, page)}
            disabled={loading}
          >
            刷新
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => void handleExport()}
            disabled={exporting || loading}
          >
            {exporting ? "导出中…" : "导出 CSV"}
          </Button>
        </div>
      </div>

      {/* 过滤条 */}
      <div className="flex items-center gap-2 flex-wrap text-sm">
        <Select
          value={filters.service}
          onValueChange={(v) => applyFilter("service", v)}
        >
          <SelectTrigger className="h-8 w-36 text-xs">
            <SelectValue placeholder="服务" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">全部服务</SelectItem>
            <SelectItem value="fanqie_promotion">番茄达人</SelectItem>
            <SelectItem value="qimao_promotion">七猫达人</SelectItem>
          </SelectContent>
        </Select>

        <Select
          value={filters.endpoint}
          onValueChange={(v) => applyFilter("endpoint", v)}
        >
          <SelectTrigger className="h-8 w-40 text-xs">
            <SelectValue placeholder="接口" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">全部接口</SelectItem>
            {Object.entries(ENDPOINT_LABELS).map(([k, v]) => (
              <SelectItem key={k} value={k}>
                {v}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>

        <Select
          value={filters.parsed_ok}
          onValueChange={(v) => applyFilter("parsed_ok", v)}
        >
          <SelectTrigger className="h-8 w-28 text-xs">
            <SelectValue placeholder="解析状态" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">全部状态</SelectItem>
            <SelectItem value="false">解析失败</SelectItem>
            <SelectItem value="true">解析成功</SelectItem>
          </SelectContent>
        </Select>

        <Select
          value={filters.acknowledged}
          onValueChange={(v) => applyFilter("acknowledged", v)}
        >
          <SelectTrigger className="h-8 w-28 text-xs">
            <SelectValue placeholder="标记状态" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">全部</SelectItem>
            <SelectItem value="false">未标记</SelectItem>
            <SelectItem value="true">已标记</SelectItem>
          </SelectContent>
        </Select>

        {data && (
          <span className="text-muted-foreground text-xs ml-1">
            共 {data.total} 条
          </span>
        )}
      </div>

      {/* 批量操作条 */}
      {selected.size > 0 && (
        <div className="flex items-center gap-2 px-3 py-2 rounded-md bg-muted text-sm">
          <span className="text-muted-foreground">已选 {selected.size} 条</span>
          <Button
            variant="outline"
            size="sm"
            className="h-7 text-xs"
            disabled={acting}
            onClick={() => void handleMark(true)}
          >
            标记为已知
          </Button>
          <Button
            variant="outline"
            size="sm"
            className="h-7 text-xs"
            disabled={acting}
            onClick={() => void handleMark(false)}
          >
            取消标记
          </Button>
          <Button
            variant="destructive"
            size="sm"
            className="h-7 text-xs"
            disabled={acting}
            onClick={() => void handleDelete()}
          >
            删除
          </Button>
        </div>
      )}

      {/* 错误 */}
      {error && (
        <p className="text-sm text-destructive text-center py-4">{error}</p>
      )}

      {/* 表格 */}
      {!error && (
        <div className="rounded-md border overflow-x-auto">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-8 px-2">
                  <Checkbox
                    checked={allPageSelected}
                    onCheckedChange={(c) => toggleAll(!!c)}
                    aria-label="全选当页"
                  />
                </TableHead>
                <TableHead className="w-32">时间</TableHead>
                <TableHead className="w-24">服务</TableHead>
                <TableHead>接口</TableHead>
                <TableHead className="w-16 text-right">HTTP</TableHead>
                <TableHead className="w-20">解析</TableHead>
                <TableHead className="w-20">标记</TableHead>
                <TableHead>错误原因</TableHead>
                <TableHead className="w-24" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {loading && (
                <TableRow>
                  <TableCell
                    colSpan={9}
                    className="text-center text-muted-foreground py-8"
                  >
                    加载中…
                  </TableCell>
                </TableRow>
              )}
              {!loading && data?.rows.length === 0 && (
                <TableRow>
                  <TableCell
                    colSpan={9}
                    className="text-center text-muted-foreground py-8"
                  >
                    暂无记录
                  </TableCell>
                </TableRow>
              )}
              {!loading &&
                data?.rows.map((row) => (
                  <TableRow
                    key={row.id}
                    className={
                      row.acknowledged ? "opacity-50" : undefined
                    }
                  >
                    <TableCell className="px-2">
                      <Checkbox
                        checked={selected.has(row.id)}
                        onCheckedChange={() => toggleOne(row.id)}
                      />
                    </TableCell>
                    <TableCell className="text-xs whitespace-nowrap">
                      {fmt(row.created_at)}
                    </TableCell>
                    <TableCell>
                      <Badge variant="outline" className="text-xs font-mono">
                        {SERVICE_LABELS[row.service] ?? row.service}
                      </Badge>
                    </TableCell>
                    <TableCell className="text-xs font-mono text-muted-foreground">
                      {ENDPOINT_LABELS[row.endpoint] ?? row.endpoint}
                    </TableCell>
                    <TableCell className="text-right">
                      {row.http_status ? (
                        <Badge
                          variant={
                            row.http_status >= 400
                              ? "destructive"
                              : "outline"
                          }
                          className="text-xs"
                        >
                          {row.http_status}
                        </Badge>
                      ) : (
                        <span className="text-xs text-muted-foreground">—</span>
                      )}
                    </TableCell>
                    <TableCell>
                      {row.parsed_ok ? (
                        <Badge variant="default" className="text-xs bg-success text-success-foreground">
                          成功
                        </Badge>
                      ) : (
                        <Badge variant="destructive" className="text-xs">
                          失败
                        </Badge>
                      )}
                    </TableCell>
                    <TableCell>
                      {row.acknowledged ? (
                        <Badge variant="outline" className="text-xs text-muted-foreground">
                          已知
                        </Badge>
                      ) : (
                        <span className="text-xs text-muted-foreground">—</span>
                      )}
                    </TableCell>
                    <TableCell className="text-xs text-destructive max-w-xs truncate">
                      {row.parse_error ?? ""}
                    </TableCell>
                    <TableCell>
                      <div className="flex items-center gap-1">
                        <Button
                          variant="ghost"
                          size="sm"
                          className="text-xs h-7 px-2"
                          onClick={() => setDetail(row)}
                        >
                          详情
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          className="text-xs h-7 px-2"
                          onClick={() => handleCopyRow(row)}
                          title="复制完整信息"
                        >
                          {copiedId === row.id ? "✓" : "复制"}
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
            </TableBody>
          </Table>
        </div>
      )}

      {/* 分页 */}
      {data && data.total > PAGE_SIZE && (
        <div className="flex items-center justify-between text-sm">
          <span className="text-muted-foreground text-xs">
            第 {page} / {totalPages} 页
          </span>
          <div className="flex gap-2">
            <Button
              variant="outline"
              size="sm"
              disabled={page <= 1 || loading}
              onClick={() => setPage((p) => Math.max(1, p - 1))}
            >
              上一页
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={page >= totalPages || loading}
              onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
            >
              下一页
            </Button>
          </div>
        </div>
      )}

      {/* 详情弹窗 */}
      <Dialog open={detail !== null} onOpenChange={() => setDetail(null)}>
        <DialogContent className="max-w-3xl max-h-[80vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle className="text-sm">
              #{detail?.id} ·{" "}
              {detail ? (SERVICE_LABELS[detail.service] ?? detail.service) : ""}
              {" / "}
              {detail
                ? (ENDPOINT_LABELS[detail.endpoint] ?? detail.endpoint)
                : ""}
            </DialogTitle>
          </DialogHeader>
          {detail && (
            <div className="space-y-4 text-xs">
              <div className="grid grid-cols-3 gap-2">
                <div>
                  <p className="text-muted-foreground mb-1">时间</p>
                  <p>{fmt(detail.created_at)}</p>
                </div>
                <div>
                  <p className="text-muted-foreground mb-1">HTTP 状态</p>
                  <p>{detail.http_status ?? "—"}</p>
                </div>
                <div>
                  <p className="text-muted-foreground mb-1">解析结果</p>
                  <p>{detail.parsed_ok ? "成功" : "失败"}</p>
                </div>
              </div>
              {detail.parse_error && (
                <div>
                  <p className="text-muted-foreground mb-1">错误原因</p>
                  <pre className="bg-muted rounded p-2 whitespace-pre-wrap break-all">
                    {detail.parse_error}
                  </pre>
                </div>
              )}
              <div>
                <p className="text-muted-foreground mb-1">请求摘要</p>
                <pre className="bg-muted rounded p-2 whitespace-pre-wrap break-all max-h-40 overflow-auto">
                  {detail.request_summary
                    ? JSON.stringify(detail.request_summary, null, 2)
                    : "—"}
                </pre>
              </div>
              <div>
                <p className="text-muted-foreground mb-1">原始响应</p>
                <pre className="bg-muted rounded p-2 whitespace-pre-wrap break-all max-h-80 overflow-auto">
                  {detail.raw_response
                    ? JSON.stringify(detail.raw_response, null, 2)
                    : "—"}
                </pre>
              </div>
              <div className="flex justify-end gap-2 pt-2">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    void kolApi
                      .markApiLog({
                        ids: [detail.id],
                        acknowledged: !detail.acknowledged,
                      })
                      .then(() => {
                        setDetail(null);
                        void load(filters, page);
                      });
                  }}
                >
                  {detail.acknowledged ? "取消标记" : "标记为已知"}
                </Button>
                <Button
                  variant="destructive"
                  size="sm"
                  onClick={() => {
                    if (!confirm("确认删除此条记录？")) return;
                    void kolApi
                      .deleteApiLog({ ids: [detail.id] })
                      .then(() => {
                        setDetail(null);
                        void load(filters, page);
                      });
                  }}
                >
                  删除
                </Button>
              </div>
            </div>
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}
