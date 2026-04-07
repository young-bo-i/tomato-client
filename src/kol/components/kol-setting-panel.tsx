"use client";

import { useEffect, useState } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import { kolApi } from "../api/client";
import { AliasType, AliasTypeLabel } from "../types";
import type { KolAccountBase, CommonSetting } from "../types";

export function KolSettingPanel() {
  const [kolAccounts, setKolAccounts] = useState<KolAccountBase[]>([]);
  const [selectedKol, setSelectedKol] = useState<number | null>(null);
  const [settings, setSettings] = useState<CommonSetting[]>([]);
  const [openPlatforms, setOpenPlatforms] = useState<number[]>([]);
  const [limits, setLimits] = useState<Record<number, number>>({});
  const [noticeEmails, setNoticeEmails] = useState<string[]>([]);
  const [newEmail, setNewEmail] = useState("");
  const [hasChild, setHasChild] = useState(false);

  useEffect(() => {
    loadData();
  }, []);

  const loadData = async () => {
    const [kols, allSettings, notices] = await Promise.all([
      kolApi.getKolBaseInfos(),
      kolApi.getAllSettings(),
      kolApi.getIncomeNotice(),
    ]);
    setKolAccounts(kols);
    setSettings(allSettings);
    setNoticeEmails(notices.map((n) => n.email));
    if (notices.length > 0) setHasChild(notices[0].has_child);
    if (kols.length > 0) setSelectedKol(kols[0].id);
  };

  useEffect(() => {
    if (!selectedKol) return;
    // Parse settings for selected KOL
    const kolSettings = settings.filter((s) => s.kol_id === selectedKol);
    const openSetting = kolSettings.find((s) => s.scene === "OpenBrushPlatform");
    if (openSetting) {
      try {
        setOpenPlatforms(JSON.parse(openSetting.setting_value));
      } catch {
        setOpenPlatforms([]);
      }
    } else {
      setOpenPlatforms([]);
    }

    const limitSettings = kolSettings.filter((s) => s.scene === "BrushLimit");
    const parsedLimits: Record<number, number> = {};
    for (const ls of limitSettings) {
      try {
        const val = JSON.parse(ls.setting_value);
        parsedLimits[val.platform] = val.limit;
      } catch {
        // skip
      }
    }
    setLimits(parsedLimits);
  }, [selectedKol, settings]);

  const togglePlatform = (platform: number) => {
    const updated = openPlatforms.includes(platform)
      ? openPlatforms.filter((p) => p !== platform)
      : [...openPlatforms, platform];
    setOpenPlatforms(updated);
  };

  const savePlatforms = async () => {
    if (!selectedKol) return;
    await kolApi.savePlatformTypes(selectedKol, openPlatforms);
  };

  const saveLimit = async (platform: number) => {
    if (!selectedKol) return;
    await kolApi.saveTypeLimit(selectedKol, platform, limits[platform] || 0);
  };

  const addEmail = async () => {
    if (!newEmail) return;
    setNoticeEmails((prev) => [...prev, newEmail]);
    await kolApi.setIncomeNotice([...noticeEmails, newEmail], hasChild);
    setNewEmail("");
  };

  return (
    <div className="space-y-4">
      {/* KOL Selector */}
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">选择 KOL 账号</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="flex gap-2 flex-wrap">
            {kolAccounts.map((kol) => (
              <Button
                key={kol.id}
                size="sm"
                variant={selectedKol === kol.id ? "default" : "outline"}
                onClick={() => setSelectedKol(kol.id)}
              >
                {kol.identity_name || kol.uid || `KOL #${kol.id}`}
              </Button>
            ))}
          </div>
        </CardContent>
      </Card>

      {selectedKol && (
        <>
          {/* Platform Toggle */}
          <Card>
            <CardHeader>
              <CardTitle className="text-sm">开放平台</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              {[AliasType.XiaoShuo, AliasType.TouTiao, AliasType.ChangTing, AliasType.WuKong].map(
                (platform) => (
                  <div key={platform} className="flex items-center gap-2">
                    <Checkbox
                      checked={openPlatforms.includes(platform)}
                      onCheckedChange={() => togglePlatform(platform)}
                    />
                    <span className="text-sm">{AliasTypeLabel[platform]}</span>
                  </div>
                ),
              )}
              <Button size="sm" onClick={savePlatforms}>
                保存平台设置
              </Button>
            </CardContent>
          </Card>

          {/* Platform Limits */}
          <Card>
            <CardHeader>
              <CardTitle className="text-sm">每日限额</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              {[AliasType.XiaoShuo, AliasType.TouTiao, AliasType.ChangTing, AliasType.WuKong].map(
                (platform) => (
                  <div key={platform} className="flex items-center gap-3">
                    <Label className="w-16 text-sm">{AliasTypeLabel[platform]}</Label>
                    <Input
                      type="number"
                      className="w-24"
                      value={limits[platform] || 0}
                      onChange={(e) =>
                        setLimits((prev) => ({
                          ...prev,
                          [platform]: parseInt(e.target.value) || 0,
                        }))
                      }
                    />
                    <Button size="sm" variant="outline" onClick={() => saveLimit(platform)}>
                      保存
                    </Button>
                  </div>
                ),
              )}
            </CardContent>
          </Card>
        </>
      )}

      {/* Income Notification */}
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">收入通知</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="flex items-center gap-2">
            <Checkbox
              checked={hasChild}
              onCheckedChange={(v) => setHasChild(!!v)}
            />
            <span className="text-sm">包含子账号</span>
          </div>
          <div className="space-y-2">
            {noticeEmails.map((email, i) => (
              <div key={i} className="text-sm text-muted-foreground">
                {email}
              </div>
            ))}
          </div>
          <div className="flex gap-2">
            <Input
              value={newEmail}
              onChange={(e) => setNewEmail(e.target.value)}
              placeholder="添加通知邮箱"
              className="flex-1"
            />
            <Button size="sm" onClick={addEmail}>
              添加
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
