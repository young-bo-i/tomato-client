"use client";

import { useEffect } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { useKolTasks } from "../hooks/use-kol-tasks";
import { AliasType, AliasTypeLabel } from "../types";

export function KolDashboard() {
  const {
    summary,
    recentTasks,
    income,
    frequency,
    loading,
    fetchDashboard,
    fetchFrequency,
  } = useKolTasks();

  useEffect(() => {
    fetchDashboard();
    fetchFrequency("10min");
  }, [fetchDashboard, fetchFrequency]);

  // Group recent tasks by platform
  const platformTotals = recentTasks.reduce(
    (acc, t) => {
      acc[t.platform] = (acc[t.platform] || 0) + t.count;
      return acc;
    },
    {} as Record<number, number>,
  );

  // Total income
  const totalIncome = income.reduce((sum, i) => sum + i.total_income, 0);

  return (
    <div className="space-y-6">
      {/* Summary Cards */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm text-muted-foreground">全部任务</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="text-2xl font-bold">{summary?.total_count ?? "-"}</div>
          </CardContent>
        </Card>
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm text-muted-foreground">今日任务</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="text-2xl font-bold">{summary?.today_count ?? "-"}</div>
          </CardContent>
        </Card>
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm text-muted-foreground">待回填</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="text-2xl font-bold">{summary?.no_callback_count ?? "-"}</div>
          </CardContent>
        </Card>
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm text-muted-foreground">总收入 (元)</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="text-2xl font-bold">
              {totalIncome ? (totalIncome / 100).toFixed(2) : "-"}
            </div>
          </CardContent>
        </Card>
      </div>

      {/* Platform Breakdown */}
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">近7天平台分布</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="grid grid-cols-4 gap-4">
            {[AliasType.XiaoShuo, AliasType.TouTiao, AliasType.ChangTing, AliasType.WuKong].map(
              (platform) => (
                <div key={platform} className="text-center">
                  <div className="text-lg font-semibold">
                    {platformTotals[platform] ?? 0}
                  </div>
                  <div className="text-xs text-muted-foreground">
                    {AliasTypeLabel[platform]}
                  </div>
                </div>
              ),
            )}
          </div>
        </CardContent>
      </Card>

      {/* Recent Tasks by Day */}
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">近7天任务趋势</CardTitle>
        </CardHeader>
        <CardContent>
          {recentTasks.length === 0 ? (
            <div className="text-sm text-muted-foreground text-center py-4">暂无数据</div>
          ) : (
            <div className="space-y-2">
              {Object.entries(
                recentTasks.reduce(
                  (acc, t) => {
                    const day = t.day;
                    if (!acc[day]) acc[day] = {};
                    acc[day][t.platform] = t.count;
                    return acc;
                  },
                  {} as Record<string, Record<number, number>>,
                ),
              )
                .sort(([a], [b]) => b.localeCompare(a))
                .map(([day, platforms]) => (
                  <div key={day} className="flex items-center gap-4 text-sm">
                    <span className="w-24 text-muted-foreground">{day}</span>
                    {[AliasType.XiaoShuo, AliasType.TouTiao, AliasType.ChangTing, AliasType.WuKong].map(
                      (p) => (
                        <span key={p} className="w-16 text-center">
                          {platforms[p] || 0}
                        </span>
                      ),
                    )}
                    <span className="font-medium">
                      {Object.values(platforms).reduce((s, v) => s + v, 0)}
                    </span>
                  </div>
                ))}
            </div>
          )}
        </CardContent>
      </Card>

      {/* Income List */}
      {income.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle className="text-sm">KOL 收入概览</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="space-y-2">
              {income.map((item) => (
                <div key={item.id} className="flex items-center justify-between text-sm">
                  <span className="text-muted-foreground">KOL #{item.kol_id}</span>
                  <div className="flex gap-4">
                    <span>总收入: ¥{(item.total_income / 100).toFixed(2)}</span>
                    <span>本月: ¥{(item.current_month_income / 100).toFixed(2)}</span>
                    <span>本周: ¥{(item.current_week_income / 100).toFixed(2)}</span>
                  </div>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>
      )}

      {/* Submission Frequency */}
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">提交频率 (10分钟粒度)</CardTitle>
        </CardHeader>
        <CardContent>
          {frequency.length === 0 ? (
            <div className="text-sm text-muted-foreground text-center py-4">暂无数据</div>
          ) : (
            <div className="flex items-end gap-1 h-32">
              {frequency.slice(0, 30).reverse().map((point, i) => {
                const maxCount = Math.max(...frequency.map((p) => p.count), 1);
                const height = (point.count / maxCount) * 100;
                return (
                  <div
                    key={i}
                    className="flex-1 bg-primary/60 rounded-t min-w-[4px]"
                    style={{ height: `${Math.max(height, 2)}%` }}
                    title={`${point.time_bucket}: ${point.count}`}
                  />
                );
              })}
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
