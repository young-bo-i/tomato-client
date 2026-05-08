"use client";

import { useKolAuth } from "../hooks/use-kol-auth";
import { KolLoginForm } from "./kol-login-form";

/**
 * Gates the entire app behind KOL server login. Until the user has a valid
 * session, children are NOT mounted — so their hooks (profile events,
 * Tauri listeners, etc.) won't fire. After login the gate unmounts itself
 * and the children render normally; logout (anywhere) brings the gate back.
 */
export function KolAuthGate({ children }: { children: React.ReactNode }) {
  const { user, loading } = useKolAuth();

  if (loading) {
    return (
      <div className="fixed inset-0 z-[9999] flex items-center justify-center bg-background">
        <div className="text-sm text-muted-foreground">加载中...</div>
      </div>
    );
  }

  if (!user) {
    return (
      <div className="fixed inset-0 z-[9999] flex items-center justify-center bg-background">
        <div className="w-full max-w-sm rounded-lg border bg-card p-6 shadow-lg">
          <h1 className="mb-1 text-lg font-semibold">登录</h1>
          <p className="mb-4 text-xs text-muted-foreground">
            请先登录 KOL 服务才能使用本应用
          </p>
          <KolLoginForm />
        </div>
      </div>
    );
  }

  return <>{children}</>;
}
