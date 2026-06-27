"use client";
import { ErrorBanner } from "./shared/error-banner";

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
import type { BrowserProfile } from "@/types";
import { kolApi } from "../api/client";
import type { QimaoBook } from "../types";
import { formatRelative } from "../lib/format";

function formatWords(text: string | null, raw: number | null): string {
  if (text && text.length > 0) return text;
  if (raw == null) return "—";
  if (raw >= 10000) return `${(raw / 10000).toFixed(1)}万字`;
  return `${raw}字`;
}

export function KolQimaoBooksPanel() {
  const [books, setBooks] = useState<QimaoBook[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Token status panel: lists every qimao profile + its token state, so
  // the admin can see at a glance whether the background refresh is
  // working and trigger a manual /signin if needed (e.g. after changing
  // the password). The actual credentials are entered in the create-
  // profile dialog and never displayed here.
  const [qimaoProfiles, setQimaoProfiles] = useState<BrowserProfile[]>([]);
  const [profilesLoading, setProfilesLoading] = useState(true);
  const [refreshingProfileId, setRefreshingProfileId] = useState<string | null>(
    null,
  );
  const [refreshMessage, setRefreshMessage] = useState<{
    profileId: string;
    kind: "ok" | "err";
    text: string;
  } | null>(null);

  const loadBooks = useCallback(async () => {
    setError(null);
    try {
      setBooks(await kolApi.listQimaoBooks());
    } catch (e) {
      setError(e instanceof Error ? e.message : "加载失败");
    } finally {
      setLoading(false);
    }
  }, []);

  const loadProfiles = useCallback(async () => {
    setProfilesLoading(true);
    try {
      // Fetch from server so qimao_token (server-managed) is always current.
      const list = await kolApi.listProfiles();
      setQimaoProfiles(list.filter((p) => p.kol_platform === "qimao"));
    } catch (e) {
      console.warn("listProfiles (server) failed:", e);
    } finally {
      setProfilesLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadBooks();
    void loadProfiles();
  }, [loadBooks, loadProfiles]);

  const handleRefreshBooks = async () => {
    setRefreshing(true);
    setError(null);
    try {
      const res = await kolApi.refreshQimaoBooks();
      if (!res.ok) {
        setError(res.error ?? "抓取失败");
      } else {
        await loadBooks();
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : "抓取失败");
    } finally {
      setRefreshing(false);
    }
  };

  const handleRefreshToken = async (profileId: string) => {
    setRefreshingProfileId(profileId);
    setRefreshMessage(null);
    try {
      const res = await kolApi.refreshQimaoToken(profileId);
      if (res.ok) {
        setRefreshMessage({ profileId, kind: "ok", text: "已刷新" });
        await loadProfiles();
      } else {
        setRefreshMessage({
          profileId,
          kind: "err",
          text: res.error ?? "失败",
        });
      }
    } catch (e) {
      setRefreshMessage({
        profileId,
        kind: "err",
        text: e instanceof Error ? e.message : "失败",
      });
    } finally {
      setRefreshingProfileId(null);
    }
  };

  const lastFetched = books[0]?.fetched_at;

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div className="min-w-0">
          <h2 className="text-lg font-semibold">七猫达人书籍</h2>
          <p className="text-xs text-muted-foreground">
            数据每日 03:30 自动抓取,也可点"立即抓取"。Server 用每个七猫 profile
            的账号密码每 12 小时刷一次 token,整个流程不需要打开 浏览器。
            {lastFetched && (
              <> · 最近抓取: {new Date(lastFetched).toLocaleString()}</>
            )}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2 shrink-0">
          <Button
            variant="outline"
            size="sm"
            onClick={() => void loadBooks()}
            disabled={loading || refreshing}
          >
            刷新列表
          </Button>
          <Button size="sm" onClick={handleRefreshBooks} disabled={refreshing}>
            {refreshing ? "抓取中..." : "立即抓取"}
          </Button>
        </div>
      </div>

      {/* === Account / token status === */}
      <div className="rounded-md border bg-muted/30 p-3 flex flex-col gap-2">
        <div className="flex items-center justify-between">
          <div className="text-sm font-medium">七猫账号 token 状态</div>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void loadProfiles()}
            disabled={profilesLoading}
          >
            刷新列表
          </Button>
        </div>
        {profilesLoading ? (
          <div className="text-xs text-muted-foreground">加载中...</div>
        ) : qimaoProfiles.length === 0 ? (
          <div className="text-xs text-muted-foreground">
            暂无七猫 profile。先在新建 profile 时选"七猫达人"并填账号密码。
          </div>
        ) : (
          <div className="flex flex-col gap-1">
            {qimaoProfiles.map((p) => {
              const hasToken = !!p.qimao_token;
              const lastErr = p.qimao_token_last_error;
              return (
                <div
                  key={p.id}
                  className="flex flex-wrap items-center gap-2 text-xs"
                >
                  <span className="font-medium min-w-[120px]">{p.name}</span>
                  <span className="text-muted-foreground min-w-[140px]">
                    {p.qimao_identifier ?? "(未配置账号)"}
                  </span>
                  {hasToken ? (
                    <Badge
                      variant="outline"
                      className="border-success/40 bg-success/10 text-success"
                    >
                      在线
                    </Badge>
                  ) : (
                    <Badge variant="destructive">无 token</Badge>
                  )}
                  <span className="text-muted-foreground">
                    上次刷新: {formatRelative(p.qimao_token_refreshed_at)}
                  </span>
                  {lastErr && (
                    <span
                      className="max-w-[280px] truncate text-destructive"
                      title={lastErr}
                    >
                      失败: {lastErr}
                    </span>
                  )}
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => void handleRefreshToken(p.id)}
                    disabled={
                      refreshingProfileId === p.id || !p.qimao_identifier
                    }
                  >
                    {refreshingProfileId === p.id ? "刷新中..." : "立即刷新"}
                  </Button>
                  {refreshMessage?.profileId === p.id && (
                    <span
                      className={
                        refreshMessage.kind === "ok"
                          ? "text-success"
                          : "text-destructive"
                      }
                    >
                      {refreshMessage.text}
                    </span>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>

      {error && (
        <ErrorBanner>{error}</ErrorBanner>
      )}

      <div className="rounded-md border overflow-x-auto">
        <Table className="min-w-[820px]">
          <TableHeader>
            <TableRow>
              <TableHead className="w-12">#</TableHead>
              <TableHead>书名</TableHead>
              <TableHead className="w-28">作者</TableHead>
              <TableHead className="w-28">分类</TableHead>
              <TableHead className="w-28 text-right">字数</TableHead>
              <TableHead className="w-20">状态</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading && books.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={6}
                  className="text-center text-muted-foreground"
                >
                  加载中...
                </TableCell>
              </TableRow>
            ) : books.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={6}
                  className="text-center text-muted-foreground"
                >
                  暂无数据 — 检查上方 token 是否已成功刷新,然后点"立即抓取"
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
                  <TableCell className="text-xs text-muted-foreground">
                    {b.first_category ?? "—"}
                    {b.second_category && (
                      <span className="text-muted-foreground/60">
                        {" / "}
                        {b.second_category}
                      </span>
                    )}
                  </TableCell>
                  <TableCell className="text-right text-xs">
                    {formatWords(b.words_num_text, b.words)}
                  </TableCell>
                  <TableCell>
                    <div className="flex flex-col gap-0.5">
                      {b.is_rights ? (
                        <Badge
                          variant="outline"
                          className="text-[10px] border-success/40 text-success"
                        >
                          有版权
                        </Badge>
                      ) : (
                        <Badge variant="outline" className="text-[10px]">
                          无版权
                        </Badge>
                      )}
                      {b.is_forbid && (
                        <Badge
                          variant="outline"
                          className="text-[10px] border-destructive/40 text-destructive"
                        >
                          禁推
                        </Badge>
                      )}
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
