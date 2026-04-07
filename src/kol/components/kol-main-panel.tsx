"use client";

import { useState, useEffect } from "react";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Button } from "@/components/ui/button";
import { KolLoginDialog } from "./kol-login-dialog";
import { KolDashboard } from "./kol-dashboard";
import { KolAccountPanel } from "./kol-account-panel";
import { KolTaskPanel } from "./kol-task-panel";
import { KolAutoGatherPanel } from "./kol-auto-gather-panel";
import { KolSettingPanel } from "./kol-setting-panel";
import { useKolAuth } from "../hooks/use-kol-auth";

export function KolMainPanel() {
  const { isLoggedIn, account, loading, logout, checkAuth } = useKolAuth();
  const [showLogin, setShowLogin] = useState(false);
  const [activeTab, setActiveTab] = useState("dashboard");

  useEffect(() => {
    if (!loading && !isLoggedIn) {
      setShowLogin(true);
    }
  }, [loading, isLoggedIn]);

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="text-muted-foreground">加载中...</div>
      </div>
    );
  }

  if (!isLoggedIn) {
    return (
      <div className="flex flex-col items-center justify-center h-64 gap-4">
        <p className="text-muted-foreground">请先登录 KOL 服务</p>
        <Button onClick={() => setShowLogin(true)}>登录</Button>
        <KolLoginDialog
          open={showLogin}
          onOpenChange={setShowLogin}
          onLoginSuccess={checkAuth}
        />
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-2 border-b">
        <div className="flex items-center gap-3">
          <span className="text-sm font-medium">
            KOL 工作台
          </span>
          <span className="text-xs text-muted-foreground">
            {account?.account_name}
          </span>
        </div>
        <Button variant="ghost" size="sm" onClick={logout}>
          退出
        </Button>
      </div>

      {/* Tabs */}
      <Tabs value={activeTab} onValueChange={setActiveTab} className="flex-1 flex flex-col">
        <TabsList className="mx-4 mt-2 w-fit">
          <TabsTrigger value="dashboard">总览</TabsTrigger>
          <TabsTrigger value="accounts">账号管理</TabsTrigger>
          <TabsTrigger value="gather">自动采集</TabsTrigger>
          <TabsTrigger value="tasks">任务数据</TabsTrigger>
          <TabsTrigger value="settings">设置</TabsTrigger>
        </TabsList>

        <div className="flex-1 overflow-auto p-4">
          <TabsContent value="dashboard" className="mt-0">
            <KolDashboard />
          </TabsContent>
          <TabsContent value="accounts" className="mt-0">
            <KolAccountPanel />
          </TabsContent>
          <TabsContent value="gather" className="mt-0">
            <KolAutoGatherPanel />
          </TabsContent>
          <TabsContent value="tasks" className="mt-0">
            <KolTaskPanel />
          </TabsContent>
          <TabsContent value="settings" className="mt-0">
            <KolSettingPanel />
          </TabsContent>
        </div>
      </Tabs>

      <KolLoginDialog
        open={showLogin}
        onOpenChange={setShowLogin}
        onLoginSuccess={checkAuth}
      />
    </div>
  );
}
