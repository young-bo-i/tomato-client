"use client";

import { useState, useCallback, useRef, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { kolApi } from "../api/client";
import type { AutoGatherConfig, GatherLog, DomConfig } from "../types";

const GATHER_CONFIG_KEY = "kol_auto_gather_config";

const DEFAULT_CONFIG: AutoGatherConfig = {
  enabled_douyin_ids: [],
  start_time: "09:00",
  end_time: "23:59",
  interval_ms: 800,
  videos_per_session: 40,
};

export function useAutoGather() {
  const [config, setConfig] = useState<AutoGatherConfig>(DEFAULT_CONFIG);
  const [isRunning, setIsRunning] = useState(false);
  const [logs, setLogs] = useState<GatherLog[]>([]);
  const abortRef = useRef<AbortController | null>(null);

  // Load config from localStorage
  useEffect(() => {
    const saved = localStorage.getItem(GATHER_CONFIG_KEY);
    if (saved) {
      try {
        setConfig({ ...DEFAULT_CONFIG, ...JSON.parse(saved) });
      } catch {
        // ignore
      }
    }
  }, []);

  const saveConfig = useCallback((newConfig: AutoGatherConfig) => {
    setConfig(newConfig);
    localStorage.setItem(GATHER_CONFIG_KEY, JSON.stringify(newConfig));
  }, []);

  const addLog = useCallback(
    (douyinId: number, nickname: string, level: GatherLog["level"], message: string) => {
      const log: GatherLog = {
        id: `${Date.now()}-${Math.random()}`,
        timestamp: new Date().toISOString(),
        douyin_id: douyinId,
        douyin_nickname: nickname,
        level,
        message,
      };
      setLogs((prev) => [log, ...prev].slice(0, 500)); // keep last 500
    },
    [],
  );

  const startGathering = useCallback(async () => {
    if (isRunning) return;

    setIsRunning(true);
    abortRef.current = new AbortController();
    setLogs([]);

    addLog(0, "系统", "info", "开始自动采集任务...");

    try {
      // Fetch DOM selectors from server
      const domConfig = await kolApi.getDouYinDom();
      addLog(0, "系统", "info", "已获取DOM配置");

      // Call Tauri command to start gathering
      await invoke("kol_start_gather", {
        config: config,
        domConfig: domConfig,
      });

      addLog(0, "系统", "info", "采集任务已启动");
    } catch (e) {
      addLog(0, "系统", "error", `启动失败: ${e}`);
      setIsRunning(false);
    }
  }, [isRunning, config, addLog]);

  const stopGathering = useCallback(async () => {
    if (!isRunning) return;

    try {
      await invoke("kol_stop_gather");
      addLog(0, "系统", "info", "采集任务已停止");
    } catch (e) {
      addLog(0, "系统", "error", `停止失败: ${e}`);
    } finally {
      abortRef.current?.abort();
      setIsRunning(false);
    }
  }, [isRunning, addLog]);

  const clearLogs = useCallback(() => {
    setLogs([]);
  }, []);

  return {
    config,
    saveConfig,
    isRunning,
    logs,
    startGathering,
    stopGathering,
    clearLogs,
    addLog,
  };
}
