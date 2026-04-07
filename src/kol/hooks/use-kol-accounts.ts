"use client";

import { useState, useCallback } from "react";
import { kolApi } from "../api/client";
import type { KolAccountBase, DouYinAccountBase, QiMaoAccount } from "../types";

export function useKolAccounts() {
  const [kolAccounts, setKolAccounts] = useState<KolAccountBase[]>([]);
  const [douyinAccounts, setDouyinAccounts] = useState<DouYinAccountBase[]>([]);
  const [loading, setLoading] = useState(false);

  const refreshKolAccounts = useCallback(async () => {
    setLoading(true);
    try {
      const accounts = await kolApi.getKolBaseInfos();
      setKolAccounts(accounts);
    } finally {
      setLoading(false);
    }
  }, []);

  const refreshDouyinAccounts = useCallback(async () => {
    setLoading(true);
    try {
      const accounts = await kolApi.getDouYinBaseAccounts();
      setDouyinAccounts(accounts);
    } finally {
      setLoading(false);
    }
  }, []);

  const refreshAll = useCallback(async () => {
    setLoading(true);
    try {
      const [kol, dy] = await Promise.all([
        kolApi.getKolBaseInfos(),
        kolApi.getDouYinBaseAccounts(),
      ]);
      setKolAccounts(kol);
      setDouyinAccounts(dy);
    } finally {
      setLoading(false);
    }
  }, []);

  const deleteKol = useCallback(async (id: number) => {
    await kolApi.deleteKolAccount(id);
    setKolAccounts((prev) => prev.filter((a) => a.id !== id));
  }, []);

  const deleteDouyin = useCallback(async (id: number) => {
    await kolApi.deleteDouYinAccount(id);
    setDouyinAccounts((prev) => prev.filter((a) => a.id !== id));
  }, []);

  return {
    kolAccounts,
    douyinAccounts,
    loading,
    refreshKolAccounts,
    refreshDouyinAccounts,
    refreshAll,
    deleteKol,
    deleteDouyin,
  };
}
