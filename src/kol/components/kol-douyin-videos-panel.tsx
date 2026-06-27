"use client";
import { ErrorBanner } from "./shared/error-banner";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import React from "react";
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
import type { BrowserProfile } from "@/types";
import { kolApi } from "../api/client";
import type { DouyinVideo } from "../types";

type DouyinProfileInfo = {
  profile: BrowserProfile;
  running: boolean;
};

const PAGE_SIZE = 200;

// Formatting helper — kept outside the component so it's not recreated each render.
function fmtTime(iso: string): string {
  return new Date(iso).toLocaleTimeString();
}

/**
 * Read-only video data view. Batch / per-profile launch controls live
 * in the "采集控制" tab so this panel stays focused on the data.
 */
export function KolDouyinVideosPanel() {
  const [videos, setVideos] = useState<DouyinVideo[]>([]);
  const [profiles, setProfiles] = useState<DouyinProfileInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [autoRefresh, setAutoRefresh] = useState(true);
  const [filterProfile, setFilterProfile] = useState<string | "">("");

  // Generation counter: discards responses that arrive after a newer request
  // has already been issued (filter change while a fetch is in flight).
  const loadGen = useRef(0);

  const profileNameById = useMemo(() => {
    const m: Record<string, string> = {};
    for (const p of profiles) m[p.profile.id] = p.profile.name;
    return m;
  }, [profiles]);

  const load = useCallback(async () => {
    const gen = ++loadGen.current;
    setRefreshing(true);
    setError(null);
    try {
      const [list, ps] = await Promise.all([
        kolApi.listDouyinVideos({
          profileId: filterProfile || undefined,
          limit: PAGE_SIZE,
        }),
        invoke<DouyinProfileInfo[]>("kol_list_douyin_profiles").catch(
          () => [] as DouyinProfileInfo[],
        ),
      ]);
      if (gen !== loadGen.current) return; // stale response — a newer request is pending
      setVideos(list);
      setProfiles(ps);
    } catch (e) {
      if (gen !== loadGen.current) return;
      setError(typeof e === "string" ? e : (e as Error).message);
    } finally {
      if (gen === loadGen.current) {
        setLoading(false);
        setRefreshing(false);
      }
    }
  }, [filterProfile]);

  // Initial fetch on mount and on filter change.
  useEffect(() => { void load(); }, [load]);
  // Poll every 5 s when autoRefresh is on; pauses when the window is hidden.
  useVisibilityInterval(load, 5_000, autoRefresh);

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div className="min-w-0">
          <h2 className="text-lg font-semibold">已采集视频</h2>
          <p className="text-xs text-muted-foreground">
            来自 <code>douyin_videos</code> 表,按入库时间倒序,最多{" "}
            {PAGE_SIZE} 条。
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <select
            className="h-8 rounded-md border bg-background px-2 text-xs min-w-0 max-w-[200px]"
            value={filterProfile}
            onChange={(e) => setFilterProfile(e.target.value)}
          >
            <option value="">全部 profile</option>
            {profiles.map((p) => (
              <option key={p.profile.id} value={p.profile.id}>
                {p.profile.name}
              </option>
            ))}
          </select>
          <label className="flex items-center gap-1 text-xs text-muted-foreground whitespace-nowrap">
            <input
              type="checkbox"
              checked={autoRefresh}
              onChange={(e) => setAutoRefresh(e.target.checked)}
            />
            自动刷新
          </label>
          <Button
            size="sm"
            variant="outline"
            onClick={() => void load()}
            disabled={refreshing}
          >
            {refreshing ? "..." : "刷新"}
          </Button>
        </div>
      </div>

      {error && (
        <ErrorBanner>{error}</ErrorBanner>
      )}

      <div className="rounded-md border overflow-x-auto">
        <Table className="min-w-[900px]">
          <TableHeader>
            <TableRow>
              <TableHead className="w-20">入库</TableHead>
              <TableHead className="w-28">Profile</TableHead>
              <TableHead className="hidden md:table-cell w-32">
                aweme_id
              </TableHead>
              <TableHead className="min-w-[180px]">title</TableHead>
              <TableHead className="w-28">title 过滤</TableHead>
              <TableHead className="hidden lg:table-cell min-w-[140px]">
                suggest_word
              </TableHead>
              <TableHead className="w-28">suggest 过滤</TableHead>
              <TableHead className="w-14 text-right">封面</TableHead>
              <TableHead className="w-14 text-right">链接</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading && videos.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={9}
                  className="text-center text-muted-foreground"
                >
                  加载中...
                </TableCell>
              </TableRow>
            ) : videos.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={9}
                  className="text-center text-muted-foreground py-8"
                >
                  暂无数据 — 切到"采集控制"开启批量或单个 profile
                </TableCell>
              </TableRow>
            ) : (
              videos.map((v) => (
                <VideoRow
                  key={v.id}
                  v={v}
                  profileName={
                    profileNameById[v.profile_id] ?? v.profile_id.slice(0, 8)
                  }
                />
              ))
            )}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}

// ── VideoRow ──────────────────────────────────────────────────────────────────
// Memoized with a custom comparator: skips re-render when id and
// title_filtered are unchanged (the two fields most likely to update
// server-side after initial insert). The 5 s poll fetches new array
// references every tick; without memo every row re-renders and
// fmtTime() is called 200× per tick even if the data is identical.
const VideoRow = React.memo(
  function VideoRow({
    v,
    profileName,
  }: {
    v: DouyinVideo;
    profileName: string;
  }) {
    return (
      <TableRow>
        <TableCell className="text-xs font-mono text-muted-foreground whitespace-nowrap">
          {fmtTime(v.inserted_at)}
        </TableCell>
        <TableCell className="text-xs truncate max-w-[120px]">
          {profileName}
        </TableCell>
        <TableCell className="hidden md:table-cell text-xs font-mono truncate max-w-[140px]">
          {v.aweme_id}
        </TableCell>
        <TableCell className="text-xs" title={v.title ?? undefined}>
          <div className="line-clamp-2 max-w-[420px]">
            {v.title ?? <span className="text-muted-foreground">—</span>}
          </div>
        </TableCell>
        <TableCell
          className="text-xs"
          title={v.title_filtered ?? undefined}
        >
          {v.title_filtered ? (
            <span className="font-medium text-success break-all">
              {v.title_filtered}
            </span>
          ) : (
            <span className="text-muted-foreground">—</span>
          )}
        </TableCell>
        <TableCell
          className="hidden lg:table-cell text-xs"
          title={v.suggest_word ?? undefined}
        >
          <div className="line-clamp-2 max-w-[200px]">
            {v.suggest_word ?? (
              <span className="text-muted-foreground">—</span>
            )}
          </div>
        </TableCell>
        <TableCell
          className="text-xs"
          title={v.suggest_word_filtered ?? undefined}
        >
          {v.suggest_word_filtered ? (
            <span className="font-medium text-success break-all">
              {v.suggest_word_filtered}
            </span>
          ) : (
            <span className="text-muted-foreground">—</span>
          )}
        </TableCell>
        <TableCell className="text-right">
          {v.first_frame_url ? (
            <a
              href={v.first_frame_url}
              target="_blank"
              rel="noopener noreferrer"
              className="text-primary text-xs underline"
            >
              看
            </a>
          ) : (
            <span className="text-muted-foreground text-xs">—</span>
          )}
        </TableCell>
        <TableCell className="text-right">
          {v.share_url ? (
            <a
              href={v.share_url}
              target="_blank"
              rel="noopener noreferrer"
              className="text-primary text-xs underline"
            >
              开
            </a>
          ) : (
            <span className="text-muted-foreground text-xs">—</span>
          )}
        </TableCell>
      </TableRow>
    );
  },
  (prev, next) =>
    prev.v.id === next.v.id &&
    prev.v.title_filtered === next.v.title_filtered &&
    prev.v.suggest_word_filtered === next.v.suggest_word_filtered &&
    prev.profileName === next.profileName,
);
