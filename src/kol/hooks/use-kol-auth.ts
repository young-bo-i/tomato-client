"use client";

import { useState, useEffect, useCallback } from "react";
import { kolApi } from "../api/client";
import type { AccountInfo, LoginRequest } from "../types";

export function useKolAuth() {
  const [isLoggedIn, setIsLoggedIn] = useState(false);
  const [account, setAccount] = useState<AccountInfo | null>(null);
  const [loading, setLoading] = useState(true);

  const checkAuth = useCallback(async () => {
    if (!kolApi.isLoggedIn) {
      setIsLoggedIn(false);
      setAccount(null);
      setLoading(false);
      return;
    }
    try {
      const info = await kolApi.getAccountInfo();
      setAccount(info);
      setIsLoggedIn(true);
    } catch {
      setIsLoggedIn(false);
      setAccount(null);
      kolApi.clearToken();
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    checkAuth();
  }, [checkAuth]);

  const login = useCallback(async (req: LoginRequest) => {
    const res = await kolApi.login(req);
    await checkAuth();
    return res;
  }, [checkAuth]);

  const logout = useCallback(() => {
    kolApi.clearToken();
    setIsLoggedIn(false);
    setAccount(null);
  }, []);

  return { isLoggedIn, account, loading, login, logout, checkAuth };
}
