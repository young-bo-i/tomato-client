"use client";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useCallback, useEffect, useState } from "react";
import { kolApi } from "../api/client";
import type { KolConfigUpdate, ProfileConfig } from "../types";

// 番茄达人：3 种 alias_type
const TOMATO_TYPES = [
  { type: 1, label: "番茄小说" },
  { type: 2, label: "番茄畅听" },
  { type: 6, label: "悟空浏览器" },
];

// 七猫达人：仅 1 种
const QIMAO_TYPES = [{ type: 1, label: "七猫小说" }];

interface SlotState {
  enabled: boolean;
  daily_limit: number;
}

type LocalConfig = Record<string, Record<string, Record<number, SlotState>>>;

function defaultSlot(): SlotState {
  return { enabled: true, daily_limit: 0 };
}

function buildLocal(profiles: ProfileConfig[]): LocalConfig {
  const out: LocalConfig = {};
  for (const p of profiles) {
    out[p.profile_id] = { tomato: {}, qimao: {} };
    const types = p.kol_platform === "tomato" ? TOMATO_TYPES : QIMAO_TYPES;
    for (const t of types) {
      const plat = p.kol_platform;
      const cfg = p.configs.find(
        (c) => c.platform === plat && c.alias_type === t.type,
      );
      out[p.profile_id][plat][t.type] = cfg
        ? { enabled: cfg.enabled, daily_limit: cfg.daily_limit }
        : defaultSlot();
    }
  }
  return out;
}

function buildUpdates(local: LocalConfig, profiles: ProfileConfig[]): KolConfigUpdate[] {
  const out: KolConfigUpdate[] = [];
  for (const p of profiles) {
    const types = p.kol_platform === "tomato" ? TOMATO_TYPES : QIMAO_TYPES;
    for (const t of types) {
      const s = local[p.profile_id]?.[p.kol_platform]?.[t.type];
      if (s) {
        out.push({
          profile_id: p.profile_id,
          platform: p.kol_platform,
          alias_type: t.type,
          ...s,
        });
      }
    }
  }
  return out;
}

// ── 主组件 ─────────────────────────────────────────────────────────────────

export function KolSubmissionConfigPanel() {
  const [profiles, setProfiles] = useState<ProfileConfig[]>([]);
  const [local, setLocal] = useState<LocalConfig>({});
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await kolApi.listKolConfig();
      setProfiles(data);
      setLocal(buildLocal(data));
      setDirty(false);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  function setSlot(
    profileId: string,
    platform: string,
    aliasType: number,
    patch: Partial<SlotState>,
  ) {
    setLocal((prev) => ({
      ...prev,
      [profileId]: {
        ...prev[profileId],
        [platform]: {
          ...prev[profileId]?.[platform],
          [aliasType]: { ...prev[profileId]?.[platform]?.[aliasType], ...patch },
        },
      },
    }));
    setDirty(true);
    setSaved(false);
  }

  async function handleSave() {
    setSaving(true);
    try {
      await kolApi.updateKolConfig(buildUpdates(local, profiles));
      setDirty(false);
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setSaving(false);
    }
  }

  const tomatoProfiles = profiles.filter((p) => p.kol_platform === "tomato");
  const qimaoProfiles = profiles.filter((p) => p.kol_platform === "qimao");

  if (loading) {
    return <div className="text-sm text-muted-foreground text-center py-8">加载中…</div>;
  }
  if (error) {
    return <div className="text-sm text-destructive text-center py-4">{error}</div>;
  }

  return (
    <div className="space-y-6">
      {/* 操作栏 */}
      <div className="flex items-center justify-between gap-2">
        <div>
          <h3 className="text-sm font-semibold">提交词配置</h3>
          <p className="text-xs text-muted-foreground mt-0.5">
            每个账号每个类型独立开关和每日限额（0 = 无限制）。词投满时溢出到管理员账号，全部满则丢弃。
          </p>
        </div>
        <div className="flex items-center gap-2 shrink-0">
          <Button variant="outline" size="sm" onClick={() => void load()} disabled={loading}>
            刷新
          </Button>
          <Button size="sm" onClick={() => void handleSave()} disabled={!dirty || saving}>
            {saving ? "保存中…" : saved ? "✓ 已保存" : "保存"}
          </Button>
        </div>
      </div>

      {/* 番茄达人区块 */}
      {tomatoProfiles.length > 0 && (
        <PlatformSection
          title="番茄达人"
          profiles={tomatoProfiles}
          platform="tomato"
          types={TOMATO_TYPES}
          local={local}
          onSetSlot={setSlot}
        />
      )}

      {/* 七猫达人区块 */}
      {qimaoProfiles.length > 0 && (
        <PlatformSection
          title="七猫达人"
          profiles={qimaoProfiles}
          platform="qimao"
          types={QIMAO_TYPES}
          local={local}
          onSetSlot={setSlot}
        />
      )}

      {profiles.length === 0 && (
        <div className="text-sm text-muted-foreground text-center py-8">
          暂无番茄达人或七猫达人账号
        </div>
      )}
    </div>
  );
}

// ── 单平台表格 ─────────────────────────────────────────────────────────────

function PlatformSection({
  title,
  profiles,
  platform,
  types,
  local,
  onSetSlot,
}: {
  title: string;
  profiles: ProfileConfig[];
  platform: string;
  types: { type: number; label: string }[];
  local: LocalConfig;
  onSetSlot: (pid: string, plat: string, at: number, patch: Partial<SlotState>) => void;
}) {
  return (
    <div className="space-y-2">
      <h4 className="text-xs font-semibold text-muted-foreground uppercase tracking-wide">
        {title}
      </h4>
      <div className="rounded-md border overflow-x-auto">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="w-32">账号</TableHead>
              <TableHead className="w-20">角色</TableHead>
              {types.map((t) => (
                <TableHead key={t.type} className="text-center w-36">
                  {t.label}
                </TableHead>
              ))}
            </TableRow>
          </TableHeader>
          <TableBody>
            {profiles.map((p) => (
              <TableRow key={p.profile_id}>
                <TableCell className="font-medium text-sm">{p.profile_name}</TableCell>
                <TableCell>
                  {p.is_admin ? (
                    <Badge variant="default" className="text-[10px]">管理员</Badge>
                  ) : (
                    <Badge variant="outline" className="text-[10px]">{p.username}</Badge>
                  )}
                </TableCell>
                {types.map((t) => {
                  const slot =
                    local[p.profile_id]?.[platform]?.[t.type] ?? defaultSlot();
                  return (
                    <TableCell key={t.type}>
                      <SlotCell
                        slot={slot}
                        onChange={(patch) =>
                          onSetSlot(p.profile_id, platform, t.type, patch)
                        }
                      />
                    </TableCell>
                  );
                })}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}

// ── 单格组件 ───────────────────────────────────────────────────────────────

function SlotCell({
  slot,
  onChange,
}: {
  slot: SlotState;
  onChange: (patch: Partial<SlotState>) => void;
}) {
  return (
    <div className="flex flex-col items-center gap-1.5">
      <Checkbox
        checked={slot.enabled}
        onCheckedChange={(v: boolean) => onChange({ enabled: v })}
      />
      {slot.enabled && (
        <input
          type="number"
          min={0}
          value={slot.daily_limit}
          onChange={(e) =>
            onChange({ daily_limit: Math.max(0, Number(e.target.value)) })
          }
          className="w-20 text-center text-xs border rounded px-1 py-0.5 bg-background"
          placeholder="0=无限"
          title="每日限额（0 = 无限制）"
        />
      )}
    </div>
  );
}
