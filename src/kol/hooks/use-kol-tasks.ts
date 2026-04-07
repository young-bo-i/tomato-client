"use client";

import { useState, useCallback } from "react";
import { kolApi } from "../api/client";
import type {
  TaskDataGrid,
  TaskSummary,
  TaskQueryRequest,
  RecentTaskPoint,
  FrequencyPoint,
  KolIncome,
} from "../types";

export function useKolTasks() {
  const [taskGrid, setTaskGrid] = useState<TaskDataGrid | null>(null);
  const [summary, setSummary] = useState<TaskSummary | null>(null);
  const [recentTasks, setRecentTasks] = useState<RecentTaskPoint[]>([]);
  const [income, setIncome] = useState<KolIncome[]>([]);
  const [frequency, setFrequency] = useState<FrequencyPoint[]>([]);
  const [loading, setLoading] = useState(false);

  const fetchTaskGrid = useCallback(async (query: TaskQueryRequest) => {
    setLoading(true);
    try {
      const data = await kolApi.getTaskDataGrid(query);
      setTaskGrid(data);
    } finally {
      setLoading(false);
    }
  }, []);

  const fetchSummary = useCallback(async () => {
    const data = await kolApi.getTaskSummary();
    setSummary(data);
  }, []);

  const fetchRecentTasks = useCallback(async () => {
    const data = await kolApi.getRecentTasks();
    setRecentTasks(data);
  }, []);

  const fetchIncome = useCallback(async () => {
    const data = await kolApi.getRecentIncome();
    setIncome(data);
  }, []);

  const fetchFrequency = useCallback(async (interval?: string) => {
    const data = await kolApi.getRequestFrequency(interval);
    setFrequency(data);
  }, []);

  const fetchDashboard = useCallback(async () => {
    setLoading(true);
    try {
      await Promise.all([fetchSummary(), fetchRecentTasks(), fetchIncome()]);
    } finally {
      setLoading(false);
    }
  }, [fetchSummary, fetchRecentTasks, fetchIncome]);

  return {
    taskGrid,
    summary,
    recentTasks,
    income,
    frequency,
    loading,
    fetchTaskGrid,
    fetchSummary,
    fetchRecentTasks,
    fetchIncome,
    fetchFrequency,
    fetchDashboard,
  };
}
