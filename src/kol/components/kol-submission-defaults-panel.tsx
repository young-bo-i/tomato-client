"use client";
import { ErrorBanner } from "./shared/error-banner";

import { useCallback, useEffect, useState } from "react";
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
import { kolApi } from "../api/client";
import type { KolConfigDefault } from "../types";

/// Admin-only panel: edit per-(platform, alias_type) DEFAULT values.
/// These defaults are read **once** when a new tomato/qimao profile is
/// created — they seed `kol_submission_config` rows. Editing here does
/// NOT touch existing per-profile rows; the previous behavior of
/// "admin edits everyone's config from one place" is gone (each user
/// now manages their own profiles via 我的提交配置).
const TOMATO_TYPES = [
  { type: 1, label: "番茄小说" },
  { type: 2, label: "番茄畅听" },
  { type: 6, label: "悟空浏览器" },
];

const QIMAO_TYPES = [{ type: 1, label: "七猫小说" }];

interface SlotState {
  enabled: boolean;
  daily_limit: number;
}

type LocalDefaults = Record<string, Record<number, SlotState>>;

function buildLocal(rows: KolConfigDefault[]): LocalDefaults {
  const out: LocalDefaults = { tomato: {}, qimao: {} };
  for (const r of rows) {
    if (!out[r.platform]) out[r.platform] = {};
    out[r.platform][r.alias_type] = {
      enabled: r.enabled,
      daily_limit: r.daily_limit,
    };
  }
  return out;
}

function buildUpdates(local: LocalDefaults): KolConfigDefault[] {
  const out: KolConfigDefault[] = [];
  for (const [platform, slots] of Object.entries(local)) {
    for (const [aliasTypeStr, slot] of Object.entries(slots)) {
      out.push({
        platform,
        alias_type: Number(aliasTypeStr),
        enabled: slot.enabled,
        daily_limit: slot.daily_limit,
        // updated_at omitted; server stamps NOW() on upsert
        updated_at: "",
      });
    }
  }
  return out;
}

export function KolSubmissionDefaultsPanel() {
  const [local, setLocal] = useState<LocalDefaults>({ tomato: {}, qimao: {} });
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const rows = await kolApi.listKolConfigDefaults();
      setLocal(buildLocal(rows));
      setDirty(false);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  function setSlot(
    platform: string,
    aliasType: number,
    patch: Partial<SlotState>,
  ) {
    setLocal((prev) => ({
      ...prev,
      [platform]: {
        ...prev[platform],
        [aliasType]: { ...prev[platform]?.[aliasType], ...patch },
      },
    }));
    setDirty(true);
    setSaved(false);
  }

  async function handleSave() {
    setSaving(true);
    setError(null);
    try {
      await kolApi.updateKolConfigDefaults(buildUpdates(local));
      setDirty(false);
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setSaving(false);
    }
  }

  if (loading) {
    return (
      <div className="text-sm text-muted-foreground text-center py-8">
        加载中…
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between gap-2">
        <div>
          <h3 className="text-sm font-semibold">默认提交词配置</h3>
          <p className="text-xs text-muted-foreground mt-0.5 leading-relaxed">
            <span className="font-medium text-foreground">仅作为新建账号的初始值</span>
            —— 用户创建一个新的番茄/七猫 profile 时,服务端会从这里复制一份配置过去。
            修改这里 <span className="font-medium text-foreground">不会回填</span>
            已存在的 profile,每个用户后续在自己的「我的提交词配置」中继续维护。
          </p>
        </div>
        <div className="flex items-center gap-2 shrink-0">
          <Button
            variant="outline"
            size="sm"
            onClick={() => void load()}
            disabled={loading}
          >
            刷新
          </Button>
          <Button
            size="sm"
            onClick={() => void handleSave()}
            disabled={!dirty || saving}
          >
            {saving ? "保存中…" : saved ? "✓ 已保存" : "保存"}
          </Button>
        </div>
      </div>

      {error && (
        <ErrorBanner>{error}</ErrorBanner>
      )}

      <DefaultsSection
        title="番茄达人"
        platform="tomato"
        types={TOMATO_TYPES}
        local={local}
        onSetSlot={setSlot}
      />
      <DefaultsSection
        title="七猫达人"
        platform="qimao"
        types={QIMAO_TYPES}
        local={local}
        onSetSlot={setSlot}
      />
    </div>
  );
}

function DefaultsSection({
  title,
  platform,
  types,
  local,
  onSetSlot,
}: {
  title: string;
  platform: string;
  types: { type: number; label: string }[];
  local: LocalDefaults;
  onSetSlot: (
    platform: string,
    aliasType: number,
    patch: Partial<SlotState>,
  ) => void;
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
              <TableHead className="w-40">类型</TableHead>
              <TableHead className="w-32 text-center">启用</TableHead>
              <TableHead className="w-40">每日限额</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {types.map((t) => {
              const slot = local[platform]?.[t.type] ?? {
                enabled: true,
                daily_limit: 0,
              };
              return (
                <TableRow key={t.type}>
                  <TableCell className="font-medium text-sm">{t.label}</TableCell>
                  <TableCell className="text-center">
                    <Checkbox
                      checked={slot.enabled}
                      onCheckedChange={(v: boolean) =>
                        onSetSlot(platform, t.type, { enabled: v })
                      }
                    />
                  </TableCell>
                  <TableCell>
                    <input
                      type="number"
                      min={0}
                      value={slot.daily_limit}
                      onChange={(e) =>
                        onSetSlot(platform, t.type, {
                          daily_limit: Math.max(0, Number(e.target.value)),
                        })
                      }
                      className="w-32 text-sm border rounded px-2 py-1 bg-background"
                      placeholder="0 = 无限制"
                      disabled={!slot.enabled}
                    />
                    <span className="ml-2 text-xs text-muted-foreground">
                      0 = 无限制
                    </span>
                  </TableCell>
                </TableRow>
              );
            })}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}
