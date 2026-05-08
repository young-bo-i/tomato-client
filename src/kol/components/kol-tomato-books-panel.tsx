"use client";

import { useCallback, useEffect, useState } from "react";
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
import type { TomatoBook } from "../types";

function formatWordNum(n: number | null): string {
  if (n == null) return "—";
  if (n >= 10000) return `${(n / 10000).toFixed(1)}万字`;
  return `${n}字`;
}

function formatIncome(n: number | null): string {
  if (n == null) return "—";
  if (n >= 10000) return `¥${(n / 10000).toFixed(1)}万`;
  return `¥${n.toLocaleString()}`;
}

function formatScore(n: number | null): string {
  if (n == null) return "—";
  return n.toFixed(1);
}

export function KolTomatoBooksPanel() {
  const [books, setBooks] = useState<TomatoBook[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      setBooks(await kolApi.listTomatoBooks());
    } catch (e) {
      setError(e instanceof Error ? e.message : "加载失败");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const handleRefresh = async () => {
    setRefreshing(true);
    setError(null);
    try {
      const res = await kolApi.refreshTomatoBooks();
      if (!res.ok) {
        setError(res.error ?? "抓取失败");
      } else {
        await load();
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : "抓取失败");
    } finally {
      setRefreshing(false);
    }
  };

  const lastFetched = books[0]?.fetched_at;

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div className="min-w-0">
          <h2 className="text-lg font-semibold">番茄达人书籍</h2>
          <p className="text-xs text-muted-foreground">
            数据每日 03:00 自动抓取,也可点"立即抓取"手动触发。
            {lastFetched && (
              <> 最近更新: {new Date(lastFetched).toLocaleString()}</>
            )}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2 shrink-0">
          <Button
            variant="outline"
            size="sm"
            onClick={() => void load()}
            disabled={loading || refreshing}
          >
            刷新列表
          </Button>
          <Button size="sm" onClick={handleRefresh} disabled={refreshing}>
            {refreshing ? "抓取中..." : "立即抓取"}
          </Button>
        </div>
      </div>

      {error && (
        <div className="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      )}

      <div className="rounded-md border overflow-x-auto">
        <Table className="min-w-[820px]">
          <TableHeader>
            <TableRow>
              <TableHead className="w-12">#</TableHead>
              <TableHead>书名</TableHead>
              <TableHead className="w-28">作者</TableHead>
              <TableHead className="w-16 text-right">评分</TableHead>
              <TableHead className="w-24 text-right">字数</TableHead>
              <TableHead className="w-16 text-right">章节</TableHead>
              <TableHead className="w-28 text-right">近期收入</TableHead>
              <TableHead>分类</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading && books.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={8}
                  className="text-center text-muted-foreground"
                >
                  加载中...
                </TableCell>
              </TableRow>
            ) : books.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={8}
                  className="text-center text-muted-foreground"
                >
                  暂无数据 — 点"立即抓取"试试
                </TableCell>
              </TableRow>
            ) : (
              books.map((b) => (
                <TableRow key={b.book_id}>
                  <TableCell className="font-mono text-xs">
                    {b.position}
                  </TableCell>
                  <TableCell className="font-medium">{b.book_name}</TableCell>
                  <TableCell className="text-xs text-muted-foreground">
                    {b.author ?? "—"}
                  </TableCell>
                  <TableCell className="text-right font-mono">
                    {formatScore(b.score)}
                  </TableCell>
                  <TableCell className="text-right text-xs">
                    {formatWordNum(b.word_num)}
                  </TableCell>
                  <TableCell className="text-right text-xs">
                    {b.chapter_num ?? "—"}
                  </TableCell>
                  <TableCell className="text-right text-xs">
                    {formatIncome(b.recent_income)}
                  </TableCell>
                  <TableCell>
                    <div className="flex flex-wrap gap-1">
                      {(b.categories ?? []).slice(0, 3).map((c) => (
                        <Badge
                          key={c.category_id}
                          variant="secondary"
                          className="px-1.5 py-0 text-[10px] leading-tight"
                        >
                          {c.category_name}
                        </Badge>
                      ))}
                    </div>
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
