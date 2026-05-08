"use client";

import { invoke } from "@tauri-apps/api/core";
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

type LoginStateValue = "authenticated" | "unauthenticated" | "unknown";

type LoginState = {
  state: LoginStateValue;
  updatedAt: string;
  url: string | null;
};

type DouyinProfileInfo = {
  profile: BrowserProfile;
  running: boolean;
  loginState: LoginState | null;
};

type GatherStats = {
  batchesReceived: number;
  rowsReceived: number;
  uploaded: number;
  duplicates: number;
  uploadErrors: number;
  /** Rows skipped by the local 24h dedup cache before they hit the
   * remote server. Each one is a saved network request. */
  dedupSkipped: number;
};

type BatchStatus = {
  state: "idle" | "running";
  /** Profiles in the CURRENT round (queue + active + completed_in_round). */
  totalProfiles: number;
  queued: number;
  active: number;
  /** Profiles finished this round. Resets on each refill. */
  completedInRound: number;
  runningBrowsers: number;
  activeGathers: number;
  /** Wall-clock when the current 4h session started. Persists across
   * auto-loop round boundaries; resets only on full-restart. */
  sessionStartedAt: string | null;
  /** Wall-clock when the current round started. */
  roundStartedAt: string | null;
  /** 1-based round counter — auto-loop is unbounded. */
  currentRound: number | null;
  /** Wall-clock when the supervisor will trigger the next 4h full-restart. */
  nextFullRestartAt: string | null;
};

/** One entry in the batch event log (from `kol_batch_events`). */
type BatchEventKind =
  | "session_start"
  | "session_stop"
  | "round_start"
  | "round_complete"
  | "full_restart_triggered"
  | "full_restart_complete"
  | "profile_start"
  | "profile_end"
  | "profile_error";

type BatchEvent = {
  id: number;
  at: string;
  kind: BatchEventKind;
  round?: number;
  profileId?: string;
  profileName?: string;
  detail?: string;
};

/**
 * "采集控制" panel — the single place where launching, batching, and
 * stopping happens. The data view (KolDouyinVideosPanel) is read-only.
 *
 * Per-profile Launch/Stop and the batch buttons share the same
 * underlying Tauri commands (kol_start_single_profile /
 * kol_stop_single_profile / kol_batch_start / kol_batch_stop). All four
 * paths flip the `should_gather` flag the extension polls, so the
 * extension never sees a divergence between "manual" and "batch" modes.
 */
export function KolDomDumpPanel() {
  const [profiles, setProfiles] = useState<DouyinProfileInfo[]>([]);
  const [stats, setStats] = useState<GatherStats | null>(null);
  const [batchStatus, setBatchStatus] = useState<BatchStatus | null>(null);
  const [events, setEvents] = useState<BatchEvent[]>([]);
  const [batchBusy, setBatchBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setError(null);
    try {
      const [list, s, bs, evs] = await Promise.all([
        invoke<DouyinProfileInfo[]>("kol_list_douyin_profiles"),
        invoke<GatherStats>("kol_gather_local_stats"),
        invoke<BatchStatus>("kol_batch_status").catch(() => null),
        invoke<BatchEvent[]>("kol_batch_events").catch(() => [] as BatchEvent[]),
      ]);
      setProfiles(list);
      setStats(s);
      if (bs) setBatchStatus(bs);
      setEvents(evs);
    } catch (e) {
      setError(typeof e === "string" ? e : (e as Error).message);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    let timer: ReturnType<typeof setInterval> | null = null;

    const start = () => {
      if (timer !== null) return;
      void refresh();
      timer = setInterval(() => void refresh(), 3000);
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

  const onBatchToggle = async () => {
    if (!batchStatus) return;
    setBatchBusy(true);
    setError(null);
    try {
      const cmd =
        batchStatus.state === "running" ? "kol_batch_stop" : "kol_batch_start";
      const next = await invoke<BatchStatus>(cmd);
      setBatchStatus(next);
      setTimeout(() => void refresh(), 1500);
    } catch (e) {
      setError(typeof e === "string" ? e : (e as Error).message);
    } finally {
      setBatchBusy(false);
    }
  };

  const startSingle = async (info: DouyinProfileInfo) => {
    setBusyId(info.profile.id);
    setError(null);
    try {
      // Same semantics as a batch worker: flip should_gather + launch.
      // The extension auto-starts gather on attach; unauth watchdog
      // takes over if login doesn't happen within 90s.
      await invoke("kol_start_single_profile", {
        profileId: info.profile.id,
      });
      await refresh();
    } catch (e) {
      setError(typeof e === "string" ? e : (e as Error).message);
    } finally {
      setBusyId(null);
    }
  };

  const stopSingle = async (info: DouyinProfileInfo) => {
    setBusyId(info.profile.id);
    setError(null);
    try {
      await invoke("kol_stop_single_profile", {
        profileId: info.profile.id,
      });
      await refresh();
    } catch (e) {
      setError(typeof e === "string" ? e : (e as Error).message);
    } finally {
      setBusyId(null);
    }
  };

  // Per-profile manual controls disable themselves while the rolling
  // pool is running — the pool already owns the should_gather flag for
  // every profile, manual toggles would race against the worker loop.
  const batchRunning = batchStatus?.state === "running";

  return (
    <div className="flex flex-col gap-4">
      <BatchControl
        status={batchStatus}
        busy={batchBusy}
        onToggle={() => void onBatchToggle()}
      />

      <BatchEventLog events={events} />

      <div className="flex flex-col gap-1">
        <h2 className="text-lg font-semibold">单 profile 控制</h2>
        <p className="text-xs text-muted-foreground leading-relaxed">
          每行的 <strong>启动并采集</strong> 按钮跟批量启动走的是同一条路径
          (设 <code>should_gather</code> 旗标 + 启动浏览器),
          浏览器开起来后扩展自动开始采集。批量在跑时单 profile 操作禁用,
          避免和 worker 池产生竞争。
        </p>
      </div>

      {stats && (
        <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-6 gap-2 text-xs">
          <StatCard label="收到 batch" value={stats.batchesReceived} />
          <StatCard label="收到 行" value={stats.rowsReceived} />
          <StatCard
            label="本地去重"
            value={stats.dedupSkipped}
            tone={stats.dedupSkipped > 0 ? "good" : undefined}
          />
          <StatCard
            label="入库"
            value={stats.uploaded}
            tone={stats.uploaded > 0 ? "good" : undefined}
          />
          <StatCard label="服务端重复" value={stats.duplicates} />
          <StatCard
            label="错误"
            value={stats.uploadErrors}
            tone={stats.uploadErrors > 0 ? "bad" : undefined}
          />
        </div>
      )}

      {error && (
        <div className="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      )}

      <div className="rounded-md border overflow-x-auto">
        <Table className="min-w-[640px]">
          <TableHeader>
            <TableRow>
              <TableHead className="min-w-[160px]">名称</TableHead>
              <TableHead className="w-20">运行</TableHead>
              <TableHead className="w-28">登录状态</TableHead>
              <TableHead className="w-44 text-right">操作</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading && profiles.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={4}
                  className="text-center text-muted-foreground"
                >
                  加载中...
                </TableCell>
              </TableRow>
            ) : profiles.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={4}
                  className="text-center text-muted-foreground py-6"
                >
                  没有抖音 profile — 在"浏览器配置"中创建一个
                  <br />
                  <span className="text-xs">
                    (browser=wayfern, kol_platform=douyin)
                  </span>
                </TableCell>
              </TableRow>
            ) : (
              profiles.map((info) => (
                <TableRow key={info.profile.id}>
                  <TableCell className="font-medium align-top">
                    <div className="flex flex-col gap-0.5">
                      <span className="truncate max-w-[260px]">
                        {info.profile.name}
                      </span>
                      <span className="text-[10px] text-muted-foreground font-mono">
                        {info.profile.id.slice(0, 8)}
                      </span>
                    </div>
                  </TableCell>
                  <TableCell className="align-top">
                    {info.running ? (
                      <Badge variant="default" className="text-[10px]">
                        运行中
                      </Badge>
                    ) : (
                      <Badge variant="secondary" className="text-[10px]">
                        未启动
                      </Badge>
                    )}
                  </TableCell>
                  <TableCell className="align-top">
                    <LoginBadge info={info} />
                  </TableCell>
                  <TableCell className="text-right align-top">
                    {info.running ? (
                      <Button
                        size="sm"
                        variant="destructive"
                        onClick={() => void stopSingle(info)}
                        disabled={
                          batchRunning || busyId === info.profile.id
                        }
                        className="whitespace-nowrap"
                        title={
                          batchRunning
                            ? "批量在跑,请先停止批量"
                            : undefined
                        }
                      >
                        {busyId === info.profile.id
                          ? "停止中..."
                          : "⏸ 停止"}
                      </Button>
                    ) : (
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={() => void startSingle(info)}
                        disabled={
                          batchRunning || busyId === info.profile.id
                        }
                        className="whitespace-nowrap"
                        title={
                          batchRunning
                            ? "批量在跑,请先停止批量"
                            : undefined
                        }
                      >
                        {busyId === info.profile.id
                          ? "启动中..."
                          : "▶️ 启动并采集"}
                      </Button>
                    )}
                  </TableCell>
                </TableRow>
              ))
            )}
          </TableBody>
        </Table>
      </div>

      <p className="text-xs text-muted-foreground">
        提示:每次重启 Donut 客户端后,扩展文件会写到{" "}
        <code>~/Library/Application Support/Donut*/kol-extension-{"{uuid}"}/</code>{" "}
        per profile。需要重启对应的 Wayfern profile 才会加载新版本扩展。
      </p>
    </div>
  );
}

/** Render HH:mm:ss for a duration in seconds (negative clamped to 0). */
function fmtDuration(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
}

function fmtClock(iso: string): string {
  const d = new Date(iso);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}:${String(d.getSeconds()).padStart(2, "0")}`;
}

function BatchControl({
  status,
  busy,
  onToggle,
}: {
  status: BatchStatus | null;
  busy: boolean;
  onToggle: () => void;
}) {
  // Drive a 1s tick for the session-age + restart-countdown displays.
  // Using state instead of an interval+ref so the component re-renders
  // smoothly while still letting `refresh()` (3s) update the underlying
  // BatchStatus.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (status?.state !== "running") return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [status?.state]);

  const running = status?.state === "running";
  const total = status?.totalProfiles ?? 0;
  const queued = status?.queued ?? 0;
  const active = status?.active ?? 0;
  const completed = status?.completedInRound ?? 0;
  const browsers = status?.runningBrowsers ?? 0;
  const gathers = status?.activeGathers ?? 0;
  const round = status?.currentRound ?? null;
  const sessionStartedAt = status?.sessionStartedAt ?? null;
  const nextRestartAt = status?.nextFullRestartAt ?? null;
  const progressPct = total > 0 ? Math.round((completed / total) * 100) : 0;

  const sessionAgeSec = sessionStartedAt
    ? (now - new Date(sessionStartedAt).getTime()) / 1000
    : 0;
  const restartInSec = nextRestartAt
    ? (new Date(nextRestartAt).getTime() - now) / 1000
    : 0;

  return (
    <div
      className={`flex flex-col gap-3 p-4 rounded-lg border ${
        running ? "border-success/40 bg-success/5" : "bg-card"
      }`}
    >
      <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
        <div className="flex flex-col gap-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="text-base font-semibold">批量采集</span>
            {running ? (
              <Badge variant="default" className="text-[10px]">
                进行中
              </Badge>
            ) : (
              <Badge variant="secondary" className="text-[10px]">
                已停止
              </Badge>
            )}
            {round !== null && (
              <span className="text-[11px] text-muted-foreground font-mono whitespace-nowrap">
                第 {round} 轮
              </span>
            )}
          </div>
          <div className="text-xs text-muted-foreground leading-relaxed">
            滚动 Worker Pool:同时跑 10 个 profile,每个 10 分钟封顶,
            队列空了自动续下一轮。每 4 小时整体重启一次以清理浏览器缓存。
            <br />
            <span className="font-mono">
              {browsers}/{total} 浏览器在跑 · {gathers} 在采集
            </span>
          </div>
        </div>
        <Button
          size="lg"
          variant={running ? "destructive" : "default"}
          onClick={onToggle}
          disabled={busy || (!running && total === 0)}
          className="md:min-w-44 self-stretch md:self-auto"
        >
          {busy
            ? running
              ? "停止中..."
              : "启动中..."
            : running
              ? "⏸ 全部停止"
              : "▶️ 批量采集开始"}
        </Button>
      </div>

      {running && (
        <>
          <div className="grid grid-cols-3 gap-2 text-xs">
            <PoolBox label="本轮队列" value={queued} />
            <PoolBox label="本轮运行中" value={active} tone="active" />
            <PoolBox label="本轮已完成" value={completed} tone="good" />
          </div>
          <div className="flex items-center gap-2">
            <div className="flex-1 h-2 rounded-full bg-muted overflow-hidden">
              <div
                className="h-full bg-success transition-all duration-500"
                style={{ width: `${progressPct}%` }}
              />
            </div>
            <span className="text-xs font-mono text-muted-foreground tabular-nums whitespace-nowrap">
              {completed}/{total}
            </span>
          </div>
          <div className="grid grid-cols-2 gap-2 text-xs">
            <ClockBox
              label="session 已运行"
              value={fmtDuration(sessionAgeSec)}
              hint={
                sessionStartedAt
                  ? `启动 ${fmtClock(sessionStartedAt)}`
                  : undefined
              }
            />
            <ClockBox
              label="距下次全量重启"
              value={fmtDuration(restartInSec)}
              hint={
                nextRestartAt ? `约 ${fmtClock(nextRestartAt)}` : undefined
              }
              tone={restartInSec < 5 * 60 ? "warning" : undefined}
            />
          </div>
        </>
      )}
    </div>
  );
}

function ClockBox({
  label,
  value,
  hint,
  tone,
}: {
  label: string;
  value: string;
  hint?: string;
  tone?: "warning";
}) {
  const toneCls =
    tone === "warning" ? "border-warning/40 bg-warning/10" : "bg-card";
  return (
    <div className={`rounded-md border ${toneCls} p-2 flex flex-col`}>
      <span className="text-[10px] text-muted-foreground">{label}</span>
      <span className="text-sm font-mono font-semibold tabular-nums">
        {value}
      </span>
      {hint && (
        <span className="text-[10px] text-muted-foreground">{hint}</span>
      )}
    </div>
  );
}

// ── Event log ──────────────────────────────────────────────────────────

const EVENT_KIND_META: Record<
  BatchEventKind,
  { label: string; icon: string; tone: "info" | "good" | "warn" | "bad" }
> = {
  session_start: { label: "session 启动", icon: "▶️", tone: "info" },
  session_stop: { label: "session 停止", icon: "⏹", tone: "info" },
  round_start: { label: "本轮开始", icon: "↻", tone: "info" },
  round_complete: { label: "本轮完成", icon: "✓", tone: "good" },
  full_restart_triggered: {
    label: "4h 全量重启",
    icon: "⚠",
    tone: "warn",
  },
  full_restart_complete: {
    label: "重启完成",
    icon: "✓",
    tone: "good",
  },
  profile_start: { label: "profile 启动", icon: "▶", tone: "info" },
  profile_end: { label: "profile 结束", icon: "·", tone: "good" },
  profile_error: { label: "错误", icon: "✗", tone: "bad" },
};

const EVENT_FILTERS: { key: "all" | "session" | "profile" | "error"; label: string }[] = [
  { key: "all", label: "全部" },
  { key: "session", label: "运维事件" },
  { key: "profile", label: "Profile 流转" },
  { key: "error", label: "仅错误" },
];

function BatchEventLog({ events }: { events: BatchEvent[] }) {
  const [filter, setFilter] = useState<"all" | "session" | "profile" | "error">(
    "all",
  );

  const filtered = events.filter((e) => {
    switch (filter) {
      case "all":
        return true;
      case "session":
        return (
          e.kind === "session_start" ||
          e.kind === "session_stop" ||
          e.kind === "round_start" ||
          e.kind === "round_complete" ||
          e.kind === "full_restart_triggered" ||
          e.kind === "full_restart_complete"
        );
      case "profile":
        return (
          e.kind === "profile_start" ||
          e.kind === "profile_end" ||
          e.kind === "profile_error"
        );
      case "error":
        return e.kind === "profile_error";
    }
  });

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center justify-between gap-2 flex-wrap">
        <h2 className="text-sm font-semibold">执行日志</h2>
        <div className="flex items-center gap-1">
          {EVENT_FILTERS.map((f) => (
            <button
              key={f.key}
              type="button"
              onClick={() => setFilter(f.key)}
              className={`text-[11px] px-2 py-0.5 rounded-md border transition-colors ${
                filter === f.key
                  ? "bg-primary text-primary-foreground border-primary"
                  : "bg-card border-border text-muted-foreground hover:bg-muted"
              }`}
            >
              {f.label}
            </button>
          ))}
          <span className="text-[10px] text-muted-foreground ml-1">
            {filtered.length}/{events.length}
          </span>
        </div>
      </div>
      <div className="rounded-md border max-h-72 overflow-y-auto">
        {filtered.length === 0 ? (
          <div className="text-center text-xs text-muted-foreground py-6">
            {events.length === 0
              ? "尚无事件 — 启动批量采集后会在这里显示"
              : "当前过滤无匹配"}
          </div>
        ) : (
          <ul className="divide-y">
            {filtered.map((e) => (
              <EventRow key={e.id} ev={e} />
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

function EventRow({ ev }: { ev: BatchEvent }) {
  const meta = EVENT_KIND_META[ev.kind];
  const toneCls =
    meta.tone === "good"
      ? "text-success"
      : meta.tone === "warn"
        ? "text-warning"
        : meta.tone === "bad"
          ? "text-destructive"
          : "text-muted-foreground";
  return (
    <li className="flex items-start gap-2 px-2 py-1.5 text-xs">
      <span className={`shrink-0 w-4 text-center ${toneCls}`}>
        {meta.icon}
      </span>
      <span className="shrink-0 font-mono text-muted-foreground tabular-nums">
        {fmtClock(ev.at)}
      </span>
      {ev.round !== undefined && (
        <span className="shrink-0 font-mono text-[10px] text-muted-foreground bg-muted rounded px-1">
          R{ev.round}
        </span>
      )}
      <span className={`shrink-0 font-medium ${toneCls}`}>{meta.label}</span>
      {ev.profileName && (
        <span className="shrink-0 truncate max-w-[200px]" title={ev.profileId}>
          {ev.profileName}
        </span>
      )}
      {ev.detail && (
        <span className="text-muted-foreground truncate" title={ev.detail}>
          {ev.detail}
        </span>
      )}
    </li>
  );
}

function PoolBox({
  label,
  value,
  tone,
}: {
  label: string;
  value: number;
  tone?: "active" | "good";
}) {
  const toneCls =
    tone === "active"
      ? "border-primary/40 bg-primary/10"
      : tone === "good"
        ? "border-success/40 bg-success/10"
        : "bg-card";
  return (
    <div className={`rounded-md border ${toneCls} p-2 flex flex-col`}>
      <span className="text-[10px] text-muted-foreground">{label}</span>
      <span className="text-base font-mono font-semibold">{value}</span>
    </div>
  );
}

function LoginBadge({ info }: { info: DouyinProfileInfo }) {
  if (!info.loginState) {
    return (
      <Badge variant="outline" className="text-[10px]">
        {info.running ? "等待上报..." : "未启动"}
      </Badge>
    );
  }
  const ageSec = Math.max(
    0,
    Math.floor(
      (Date.now() - new Date(info.loginState.updatedAt).getTime()) / 1000,
    ),
  );
  const stale = ageSec > 30;
  switch (info.loginState.state) {
    case "authenticated":
      return (
        <Badge
          variant="default"
          className={`text-[10px] ${stale ? "opacity-60" : ""}`}
          title={`updated ${ageSec}s ago`}
        >
          ✓ 已登录{stale ? ` (${ageSec}s)` : ""}
        </Badge>
      );
    case "unauthenticated":
      return (
        <Badge
          variant="destructive"
          className="text-[10px]"
          title={`updated ${ageSec}s ago`}
        >
          ⚠ 未登录
        </Badge>
      );
    default:
      return (
        <Badge variant="secondary" className="text-[10px]">
          ? 未知
        </Badge>
      );
  }
}

function StatCard({
  label,
  value,
  tone,
}: {
  label: string;
  value: number;
  tone?: "good" | "bad";
}) {
  const toneCls =
    tone === "good"
      ? "border-success/40 bg-success/10"
      : tone === "bad"
        ? "border-destructive/40 bg-destructive/10"
        : "bg-card";
  return (
    <div className={`rounded-md border ${toneCls} p-2 flex flex-col`}>
      <span className="text-[10px] text-muted-foreground">{label}</span>
      <span className="text-base font-mono font-semibold">{value}</span>
    </div>
  );
}
