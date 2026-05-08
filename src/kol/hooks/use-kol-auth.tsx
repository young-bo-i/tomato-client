"use client";

import { invoke } from "@tauri-apps/api/core";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";
import { kolApi } from "../api/client";
import type { LoginRequest, User } from "../types";

interface KolAuthValue {
  user: User | null;
  loading: boolean;
  isLoggedIn: boolean;
  isAdmin: boolean;
  login: (req: LoginRequest) => Promise<void>;
  logout: () => void;
  refresh: () => Promise<void>;
}

const KolAuthContext = createContext<KolAuthValue | null>(null);

// Forward the current session to the Tauri Rust side so profile commands can
// reach our server. No-op when running in a plain browser (the Tauri global
// isn't available) — we just swallow the error.
async function pushCredsToTauri() {
  const token = kolApi.getToken();
  if (!token) return;
  try {
    await invoke("set_kol_credentials", {
      args: { serverUrl: kolApi.getServerUrl(), token },
    });
  } catch (e) {
    console.warn("set_kol_credentials (tauri) failed:", e);
  }
}

async function clearCredsInTauri() {
  try {
    await invoke("clear_kol_credentials");
  } catch (e) {
    console.warn("clear_kol_credentials (tauri) failed:", e);
  }
}

export function KolAuthProvider({ children }: { children: React.ReactNode }) {
  const [user, setUser] = useState<User | null>(null);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    if (!kolApi.isLoggedIn) {
      setUser(null);
      setLoading(false);
      await clearCredsInTauri();
      return;
    }
    try {
      const me = await kolApi.me();
      setUser(me);
      await pushCredsToTauri();
    } catch {
      setUser(null);
      kolApi.clearToken();
      await clearCredsInTauri();
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Periodic /me refresh so backend-side hierarchy changes show up
  // without requiring a manual logout. The main thing that changes
  // is `has_subordinates` (admin promoted someone to be the caller's
  // tier-2 subordinate) — that needs to flip the team-management nav
  // visible without a full re-login. Tab-visibility-aware so a
  // backgrounded window doesn't burn the server with /me hits.
  //
  // 5 min cadence: `has_subordinates` changes are rare (operator
  // creates/deletes a user) and not time-critical. Faster polling
  // would mostly waste round trips.
  useEffect(() => {
    const REFRESH_INTERVAL_MS = 5 * 60 * 1000;
    let timer: ReturnType<typeof setInterval> | null = null;

    const start = () => {
      if (timer !== null) return;
      timer = setInterval(() => void refresh(), REFRESH_INTERVAL_MS);
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

    if (typeof document !== "undefined" && document.visibilityState === "visible") {
      start();
    }
    if (typeof document !== "undefined") {
      document.addEventListener("visibilitychange", onVisibility);
    }
    return () => {
      if (typeof document !== "undefined") {
        document.removeEventListener("visibilitychange", onVisibility);
      }
      stop();
    };
  }, [refresh]);

  const login = useCallback(async (req: LoginRequest) => {
    const res = await kolApi.login(req);
    await pushCredsToTauri();
    setUser(res.user);
    setLoading(false);
  }, []);

  const logout = useCallback(async () => {
    kolApi.clearToken();
    await clearCredsInTauri();
    setUser(null);
  }, []);

  const value = useMemo<KolAuthValue>(
    () => ({
      user,
      loading,
      isLoggedIn: user !== null,
      isAdmin: user?.role === "admin",
      login,
      logout,
      refresh,
    }),
    [user, loading, login, logout, refresh],
  );

  return (
    <KolAuthContext.Provider value={value}>{children}</KolAuthContext.Provider>
  );
}

export function useKolAuth(): KolAuthValue {
  const ctx = useContext(KolAuthContext);
  if (!ctx) {
    throw new Error("useKolAuth must be used inside <KolAuthProvider>");
  }
  return ctx;
}
