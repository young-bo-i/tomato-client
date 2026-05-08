"use client";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useState } from "react";
import { useKolAuth } from "../hooks/use-kol-auth";
import { KolAdminPanel } from "./kol-admin-panel";
import { KolApiLogPanel } from "./kol-api-log-panel";
import { KolEmailSettingsPanel } from "./kol-email-settings-panel";
import { KolGlobalSettingsPanel } from "./kol-global-settings-panel";
import { KolJobsPanel } from "./kol-jobs-panel";
import { KolPasswordChangeButton } from "./kol-password-change-dialog";
import { KolQimaoBooksPanel } from "./kol-qimao-books-panel";
import { KolSideNav, type NavGroup } from "./kol-side-nav";
import { KolSubmissionDefaultsPanel } from "./kol-submission-defaults-panel";
import { KolTomatoBooksPanel } from "./kol-tomato-books-panel";

const ADMIN_GROUPS: NavGroup[] = [
  {
    label: "提交配置",
    items: [
      // 注意:这里只编辑 (platform, alias_type) 维度的「默认值」,
      // 仅作用于新建 profile 时的初始值。每个用户的具体 profile 配置
      // 由用户自己在主面板「我的提交词配置」里维护。
      { value: "submission-defaults", label: "默认提交配置" },
      { value: "global-settings", label: "全局设置" },
    ],
  },
  {
    label: "平台数据",
    items: [
      // 番茄/七猫收益已移到主面板「我的账号池」,所有用户都能看自己的;
      // 管理员要看全员视角走 [管理员速览] 邮件 digest。
      { value: "tomato-books", label: "番茄书籍" },
      { value: "qimao-books", label: "七猫书籍" },
    ],
  },
  {
    label: "运维监控",
    items: [
      { value: "jobs", label: "定时任务" },
      { value: "api-log", label: "接口日志" },
    ],
  },
  {
    label: "账号管理",
    items: [
      { value: "users", label: "用户管理" },
      { value: "email-settings", label: "邮件设置" },
    ],
  },
];

export function KolAdminConfigPanel() {
  const { user, isAdmin, logout } = useKolAuth();
  const [active, setActive] = useState("submission-defaults");

  if (!user) return null;

  if (!isAdmin) {
    return (
      <div className="flex items-center justify-center h-full text-sm text-muted-foreground">
        无权限访问
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center justify-between gap-2 px-4 py-2 border-b shrink-0">
        <div className="flex items-center gap-3 min-w-0">
          <span className="text-sm font-medium whitespace-nowrap">管理员配置</span>
          <span className="text-xs text-muted-foreground truncate">{user.username}</span>
          <Badge variant="default" className="text-[10px] shrink-0">管理员</Badge>
        </div>
        <div className="flex items-center gap-1 shrink-0">
          <KolPasswordChangeButton />
          <Button variant="ghost" size="sm" onClick={logout}>
            退出
          </Button>
        </div>
      </div>

      {/* Body: sidebar + content */}
      <div className="flex flex-1 min-h-0">
        <KolSideNav groups={ADMIN_GROUPS} active={active} onChange={setActive} />
        <div className="flex-1 overflow-auto p-4">
          {active === "submission-defaults" && <KolSubmissionDefaultsPanel />}
          {active === "global-settings" && <KolGlobalSettingsPanel />}
          {active === "tomato-books" && <KolTomatoBooksPanel />}
          {active === "qimao-books" && <KolQimaoBooksPanel />}
          {active === "jobs" && <KolJobsPanel />}
          {active === "api-log" && <KolApiLogPanel />}
          {active === "users" && <KolAdminPanel currentUserId={user.id} />}
          {active === "email-settings" && <KolEmailSettingsPanel />}
        </div>
      </div>
    </div>
  );
}
