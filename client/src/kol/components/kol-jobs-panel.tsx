"use client";

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
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useCallback, useEffect, useState } from "react";
import { kolApi } from "../api/client";
import type { JobRun, JobSummary } from "../types";

interface JobMeta {
  label: string;
  schedule: string;
  kind: "cron" | "worker";
  description: string;
}

// Canonical order and metadata for every background task.
const JOB_DEFS: { name: string; meta: JobMeta }[] = [
  {
    name: "tomato_rank",
    meta: {
      label: "番茄达人榜单",
      schedule: "每日 03:00",
      kind: "cron",
      description: "从番茄达人平台抓取书籍排行榜 (~100 条)，替换 tomato_books 表。",
    },
  },
  {
    name: "qimao_rank",
    meta: {
      label: "七猫达人榜单",
      schedule: "每日 03:30",
      kind: "cron",
      description: "从七猫达人平台抓取书籍推荐列表，替换 qimao_books 表。",
    },
  },
  {
    name: "audit_log_gc",
    meta: {
      label: "审计日志清理",
      schedule: "每日 04:00",
      kind: "cron",
      description: "清理 external_api_responses（30 天）、douyin_videos（60 天）、已失败别名（30 天）、job_runs（90 天）。",
    },
  },
  {
    name: "compensation",
    meta: {
      label: "补偿任务",
      schedule: "每 30 分钟",
      kind: "cron",
      description: "每 30 分钟检查当天各 daily 任务是否已执行，服务器重启后自动补跑漏掉的任务。",
    },
  },
  {
    name: "qimao_token_refresh",
    meta: {
      label: "七猫 Token 刷新",
      schedule: "每 30 分钟",
      kind: "worker",
      description: "对 token 缺失或超过 12 小时未刷新的七猫账号重新执行 /user/signin，保持 x-qm-devops-token 有效。",
    },
  },
  {
    name: "alias_submitter",
    meta: {
      label: "番茄别名提交",
      schedule: "每 2 秒",
      kind: "worker",
      description: "消费 tomato_aliases 中 status='pending' 的行，调用番茄推广 plan/create 接口提交关键词。",
    },
  },
  {
    name: "backfill_submitter",
    meta: {
      label: "番茄链接回填",
      schedule: "每 30 秒",
      kind: "worker",
      description: "对审核通过的番茄别名执行 post/create（首次）或续期（二次），写入推广链接。",
    },
  },
  {
    name: "qimao_alias_submitter",
    meta: {
      label: "七猫别名提交",
      schedule: "每 2 秒",
      kind: "worker",
      description: "消费 qimao_aliases 中 status='pending' 的行，调用七猫推广接口提交关键词。",
    },
  },
  {
    name: "qimao_backfill_submitter",
    meta: {
      label: "七猫链接回填",
      schedule: "每 30 秒",
      kind: "worker",
      description: "轮询七猫 keyword_page 获取 alias_id，再调用 add_keyword_links 完成链接绑定。",
    },
  },
  {
    name: "notification_dispatcher",
    meta: {
      label: "离线通知推送",
      schedule: "每 60 秒",
      kind: "worker",
      description: "扫描新离线的番茄账号，通过 SMTP 向绑定邮箱发送离线告警邮件。",
    },
  },
];

const JOB_META: Record<string, JobMeta> = Object.fromEntries(
  JOB_DEFS.map(({ name, meta }) => [name, meta]),
);

function fmt(date: string | null | undefined): string {
  if (!date) return "—";
  return new Date(date).toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function fmtDuration(ms: number | null | undefined): string {
  if (ms == null) return "—";
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function SuccessRate({ successful, total }: { successful: number; total: number }) {
  if (total === 0) return <span className="text-muted-foreground">—</span>;
  const rate = Math.round((successful / total) * 100);
  const color =
    rate === 100 ? "text-success" : rate >= 80 ? "text-warning" : "text-destructive";
  return <span className={color}>{rate}%</span>;
}

// ── Stats card strip ──────────────────────────────────────────────────────────

function StatCard({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-0.5 rounded-md border bg-muted/30 px-3 py-2 min-w-[90px]">
      <span className="text-[10px] text-muted-foreground uppercase tracking-wide">{label}</span>
      <span className="text-sm font-semibold">{value}</span>
    </div>
  );
}

// ── Per-job tab content ───────────────────────────────────────────────────────

function JobTabContent({
  name,
  summary,
}: {
  name: string;
  summary: JobSummary | undefined;
}) {
  const meta = JOB_META[name]!;
  const [history, setHistory] = useState<JobRun[]>([]);
  const [histLoading, setHistLoading] = useState(false);
  const [histError, setHistError] = useState<string | null>(null);

  const loadHistory = useCallback(async () => {
    setHistLoading(true);
    setHistError(null);
    try {
      setHistory(await kolApi.getJobHistory(name, 50));
    } catch (e) {
      setHistError(e instanceof Error ? e.message : "加载失败");
    } finally {
      setHistLoading(false);
    }
  }, [name]);

  // TabsContent (Radix) only mounts children when the tab is active,
  // so a plain on-mount effect is equivalent to "load when tab opens".
  useEffect(() => {
    void loadHistory();
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="space-y-4">
      {/* Header */}
      <div className="flex flex-wrap items-start gap-2 justify-between">
        <div className="space-y-1">
          <div className="flex items-center gap-2 flex-wrap">
            <h3 className="text-sm font-semibold">{meta.label}</h3>
            <Badge variant="outline" className="text-[10px]">
              {meta.schedule}
            </Badge>
            <Badge
              variant={meta.kind === "cron" ? "default" : "secondary"}
              className="text-[10px]"
            >
              {meta.kind === "cron" ? "定时任务" : "持续 Worker"}
            </Badge>
            {summary && (
              <Badge
                variant={
                  summary.last_success === null
                    ? "outline"
                    : summary.last_success
                    ? "default"
                    : "destructive"
                }
                className={
                  summary.last_success
                    ? "text-[10px] bg-success text-success-foreground"
                    : "text-[10px]"
                }
              >
                {summary.last_success === null
                  ? "未执行"
                  : summary.last_success
                  ? "最近成功"
                  : "最近失败"}
              </Badge>
            )}
          </div>
          <p className="text-xs text-muted-foreground max-w-xl">{meta.description}</p>
        </div>
        <Button variant="ghost" size="sm" onClick={() => void loadHistory()}>
          刷新历史
        </Button>
      </div>

      {/* Stats strip */}
      {summary ? (
        <div className="flex flex-wrap gap-2">
          <StatCard label="总次数" value={summary.total_runs} />
          <StatCard
            label="成功"
            value={<span className="text-success">{summary.successful_runs}</span>}
          />
          <StatCard
            label="失败"
            value={
              summary.failed_runs > 0 ? (
                <span className="text-destructive">{summary.failed_runs}</span>
              ) : (
                "0"
              )
            }
          />
          <StatCard
            label="成功率"
            value={
              <SuccessRate
                successful={summary.successful_runs}
                total={summary.total_runs}
              />
            }
          />
          <StatCard label="平均耗时" value={fmtDuration(summary.avg_duration_ms)} />
          <StatCard label="最后执行" value={fmt(summary.last_ran_at)} />
        </div>
      ) : (
        <div className="rounded-md border bg-muted/30 px-4 py-3 text-xs text-muted-foreground">
          暂无执行记录 — Worker 跑完第一轮后会在这里显示。
        </div>
      )}

      {/* History table */}
      {histError && (
        <div className="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          加载历史失败: {histError}
        </div>
      )}
      <div className="rounded-md border overflow-x-auto">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="w-40">执行时间</TableHead>
              <TableHead className="text-right w-20">处理量</TableHead>
              <TableHead className="text-right w-20">耗时</TableHead>
              <TableHead className="w-16">状态</TableHead>
              <TableHead>错误信息</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {histLoading ? (
              <TableRow>
                <TableCell colSpan={5} className="text-center text-muted-foreground py-6">
                  加载中…
                </TableCell>
              </TableRow>
            ) : history.length === 0 ? (
              <TableRow>
                <TableCell colSpan={5} className="text-center text-muted-foreground py-6">
                  暂无记录
                </TableCell>
              </TableRow>
            ) : (
              history.map((row) => (
                <TableRow key={row.id}>
                  <TableCell className="text-xs font-mono">{fmt(row.ran_at)}</TableCell>
                  <TableCell className="text-right text-xs">{row.items_processed}</TableCell>
                  <TableCell className="text-right text-xs">
                    {fmtDuration(row.duration_ms)}
                  </TableCell>
                  <TableCell>
                    {row.success ? (
                      <Badge
                        variant="default"
                        className="text-[10px] bg-success text-success-foreground"
                      >
                        成功
                      </Badge>
                    ) : (
                      <Badge variant="destructive" className="text-[10px]">
                        失败
                      </Badge>
                    )}
                  </TableCell>
                  <TableCell
                    className="text-xs text-destructive max-w-xs truncate"
                    title={row.error_reason ?? ""}
                  >
                    {row.error_reason ?? ""}
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

// ── Main panel ────────────────────────────────────────────────────────────────

export function KolJobsPanel() {
  const [summaries, setSummaries] = useState<JobSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState(JOB_DEFS[0]!.name);


  const loadSummaries = useCallback(async () => {
    try {
      setError(null);
      setSummaries(await kolApi.listJobs());
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadSummaries();
    const t = setInterval(() => void loadSummaries(), 15_000);
    return () => clearInterval(t);
  }, [loadSummaries]);

  const summaryMap = Object.fromEntries(summaries.map((s) => [s.job_name, s]));

  if (loading) {
    return (
      <div className="text-sm text-muted-foreground text-center py-8">加载中…</div>
    );
  }

  if (error) {
    return (
      <div className="text-sm text-destructive text-center py-8">{error}</div>
    );
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold">定时任务 & Worker</h3>
        <Button variant="ghost" size="sm" onClick={() => void loadSummaries()}>
          刷新概览
        </Button>
      </div>

      <Tabs value={activeTab} onValueChange={setActiveTab}>
        {/* Tab bar — horizontally scrollable so all 10 tabs fit */}
        <div className="overflow-x-auto pb-px">
          <TabsList className="inline-flex w-max gap-0.5">
            {JOB_DEFS.map(({ name, meta }) => {
              const s = summaryMap[name];
              return (
                <TabsTrigger key={name} value={name} className="relative text-xs px-3 py-1.5">
                  {meta.label}
                  {s && s.last_success === false && (
                    <span className="absolute -top-0.5 -right-0.5 h-1.5 w-1.5 rounded-full bg-destructive" />
                  )}
                </TabsTrigger>
              );
            })}
          </TabsList>
        </div>

        {JOB_DEFS.map(({ name }) => (
          <TabsContent key={name} value={name} className="mt-3">
            <JobTabContent
              name={name}
              summary={summaryMap[name]}
            />
          </TabsContent>
        ))}
      </Tabs>
    </div>
  );
}
