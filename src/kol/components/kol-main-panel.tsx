"use client";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useMemo, useState } from "react";
import { useKolAuth } from "../hooks/use-kol-auth";
import { KolDomDumpPanel } from "./kol-dom-dump-panel";
import { KolDouyinVideosPanel } from "./kol-douyin-videos-panel";
import { KolIncomePanel } from "./kol-income-panel";
import { KolPasswordChangeButton } from "./kol-password-change-dialog";
import { KolQimaoNoticePanel } from "./kol-qimao-notice-panel";
import { KolQimaoStatsPanel } from "./kol-qimao-stats-panel";
import { KolSideNav, type NavGroup } from "./kol-side-nav";
import { KolSubmissionConfigPanel } from "./kol-submission-config-panel";
import { KolTeamSettingsPanel } from "./kol-team-settings-panel";
import { KolTomatoStatsPanel } from "./kol-tomato-stats-panel";

const COMMON_GROUPS: NavGroup[] = [
  {
    label: "采集",
    items: [
      { value: "dom-dump", label: "采集控制" },
      { value: "douyin-videos", label: "抖音视频" },
    ],
  },
  {
    label: "我的账号池",
    items: [
      { value: "my-income", label: "番茄收益" },
      { value: "my-qimao-notice", label: "七猫收益通知" },
    ],
  },
  {
    label: "我的配置",
    items: [{ value: "my-submission-config", label: "我的提交词配置" }],
  },
];

/// Tier-1 users WITH at least one tier-2 subordinate see this group.
/// Anyone else (admins, tier-2, tier-1 without subs) doesn't — the
/// setting would have no effect for them.
const TEAM_GROUPS: NavGroup[] = [
  {
    label: "团队管理",
    items: [{ value: "team-contribution", label: "二级贡献度" }],
  },
];

const ADMIN_GROUPS: NavGroup[] = [
  {
    label: "数据看板",
    items: [
      { value: "tomato-stats", label: "番茄看板" },
      { value: "qimao-stats", label: "七猫看板" },
    ],
  },
];

export function KolMainPanel() {
  const { user, isAdmin, logout } = useKolAuth();
  const [active, setActive] = useState("dom-dump");

  // The "team-management" group is only relevant when the caller has
  // tier-2 subordinates AND is themselves not admin (admins manage
  // everything via the admin-config tab). Computed before the
  // null-guard so the hook order stays stable regardless of auth state.
  const showTeamGroup = useMemo(
    () => Boolean(user?.has_subordinates) && !isAdmin,
    [user?.has_subordinates, isAdmin],
  );

  if (!user) return null;

  const groups: NavGroup[] = [
    ...COMMON_GROUPS,
    ...(showTeamGroup ? TEAM_GROUPS : []),
    ...(isAdmin ? ADMIN_GROUPS : []),
  ];

  // Tier badge surfaces the user's place in the hierarchy. Admin >
  // tier-2 (parent visible) > tier-1 (default — no extra badge to
  // avoid clutter for the common case).
  const tierBadge = (() => {
    if (isAdmin) {
      return <Badge variant="default" className="text-[10px] shrink-0">管理员</Badge>;
    }
    if (user.parent_user_id !== null) {
      return (
        <Badge
          variant="secondary"
          className="text-[10px] shrink-0"
          title={`上级: ${user.parent_username ?? user.parent_user_id}`}
        >
          二级 · {user.parent_username ?? "?"}
        </Badge>
      );
    }
    return null;
  })();

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center justify-between gap-2 px-4 py-2 border-b shrink-0">
        <div className="flex items-center gap-3 min-w-0">
          <span className="text-sm font-medium whitespace-nowrap">KOL 工作台</span>
          <span className="text-xs text-muted-foreground truncate">{user.username}</span>
          {tierBadge}
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
        <KolSideNav groups={groups} active={active} onChange={setActive} />
        <div className="flex-1 overflow-auto p-4">
          {active === "dom-dump" && <KolDomDumpPanel />}
          {active === "douyin-videos" && <KolDouyinVideosPanel />}
          {active === "my-income" && <KolIncomePanel />}
          {active === "my-qimao-notice" && <KolQimaoNoticePanel />}
          {active === "my-submission-config" && <KolSubmissionConfigPanel />}
          {showTeamGroup && active === "team-contribution" && (
            <KolTeamSettingsPanel />
          )}
          {isAdmin && active === "tomato-stats" && <KolTomatoStatsPanel />}
          {isAdmin && active === "qimao-stats" && <KolQimaoStatsPanel />}
        </div>
      </div>
    </div>
  );
}
