"use client";

import { useEffect, useState } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useAutoGather } from "../hooks/use-auto-gather";
import { useKolAccounts } from "../hooks/use-kol-accounts";

export function KolAutoGatherPanel() {
  const {
    config,
    saveConfig,
    isRunning,
    logs,
    startGathering,
    stopGathering,
    clearLogs,
  } = useAutoGather();
  const { douyinAccounts, refreshDouyinAccounts } = useKolAccounts();

  useEffect(() => {
    refreshDouyinAccounts();
  }, [refreshDouyinAccounts]);

  const toggleDouyinId = (id: number) => {
    const ids = config.enabled_douyin_ids.includes(id)
      ? config.enabled_douyin_ids.filter((i) => i !== id)
      : [...config.enabled_douyin_ids, id];
    saveConfig({ ...config, enabled_douyin_ids: ids });
  };

  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
      {/* Configuration */}
      <div className="space-y-4">
        <Card>
          <CardHeader>
            <CardTitle className="text-sm flex items-center justify-between">
              采集配置
              {isRunning && <Badge variant="default">运行中</Badge>}
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            {/* DouYin Account Selection */}
            <div className="space-y-2">
              <Label>选择抖音账号</Label>
              <div className="border rounded-md p-3 space-y-2 max-h-48 overflow-auto">
                {douyinAccounts.length === 0 ? (
                  <div className="text-sm text-muted-foreground">暂无可用抖音账号</div>
                ) : (
                  douyinAccounts
                    .filter((a) => a.status === 0)
                    .map((account) => (
                      <div key={account.id} className="flex items-center gap-2">
                        <Checkbox
                          checked={config.enabled_douyin_ids.includes(account.id)}
                          onCheckedChange={() => toggleDouyinId(account.id)}
                          disabled={isRunning}
                        />
                        <span className="text-sm">
                          {account.nickname || `抖音 #${account.id}`}
                        </span>
                      </div>
                    ))
                )}
              </div>
            </div>

            {/* Time Range */}
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label>开始时间</Label>
                <Input
                  type="time"
                  value={config.start_time}
                  onChange={(e) => saveConfig({ ...config, start_time: e.target.value })}
                  disabled={isRunning}
                />
              </div>
              <div className="space-y-2">
                <Label>结束时间</Label>
                <Input
                  type="time"
                  value={config.end_time}
                  onChange={(e) => saveConfig({ ...config, end_time: e.target.value })}
                  disabled={isRunning}
                />
              </div>
            </div>

            {/* Advanced */}
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label>视频间隔 (ms)</Label>
                <Input
                  type="number"
                  value={config.interval_ms}
                  onChange={(e) =>
                    saveConfig({ ...config, interval_ms: parseInt(e.target.value) || 800 })
                  }
                  disabled={isRunning}
                />
              </div>
              <div className="space-y-2">
                <Label>每轮视频数</Label>
                <Input
                  type="number"
                  value={config.videos_per_session}
                  onChange={(e) =>
                    saveConfig({
                      ...config,
                      videos_per_session: parseInt(e.target.value) || 40,
                    })
                  }
                  disabled={isRunning}
                />
              </div>
            </div>

            {/* Controls */}
            <div className="flex gap-2">
              {isRunning ? (
                <Button variant="destructive" className="flex-1" onClick={stopGathering}>
                  停止采集
                </Button>
              ) : (
                <Button
                  className="flex-1"
                  onClick={startGathering}
                  disabled={config.enabled_douyin_ids.length === 0}
                >
                  开始采集
                </Button>
              )}
            </div>
          </CardContent>
        </Card>
      </div>

      {/* Logs */}
      <Card className="flex flex-col">
        <CardHeader className="flex-row items-center justify-between pb-2">
          <CardTitle className="text-sm">采集日志</CardTitle>
          <Button size="sm" variant="ghost" onClick={clearLogs}>
            清空
          </Button>
        </CardHeader>
        <CardContent className="flex-1 p-0">
          <ScrollArea className="h-[500px]">
            <div className="p-3 space-y-1 font-mono text-xs">
              {logs.length === 0 ? (
                <div className="text-muted-foreground text-center py-8">等待采集...</div>
              ) : (
                logs.map((log) => (
                  <div
                    key={log.id}
                    className={`flex gap-2 ${
                      log.level === "error"
                        ? "text-destructive"
                        : log.level === "warn"
                          ? "text-yellow-600"
                          : "text-muted-foreground"
                    }`}
                  >
                    <span className="shrink-0 w-20">
                      {new Date(log.timestamp).toLocaleTimeString()}
                    </span>
                    <span className="shrink-0 w-24 truncate">
                      [{log.douyin_nickname}]
                    </span>
                    <span>{log.message}</span>
                  </div>
                ))
              )}
            </div>
          </ScrollArea>
        </CardContent>
      </Card>
    </div>
  );
}
