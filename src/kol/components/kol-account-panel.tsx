"use client";

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useKolAccounts } from "../hooks/use-kol-accounts";
import { kolApi } from "../api/client";

export function KolAccountPanel() {
  const {
    kolAccounts,
    douyinAccounts,
    loading,
    refreshAll,
    deleteKol,
    deleteDouyin,
  } = useKolAccounts();

  const [tab, setTab] = useState("kol");

  useEffect(() => {
    refreshAll();
  }, [refreshAll]);

  // Launch browser profile for KOL login via Donut Browser
  const handleAddKol = async () => {
    try {
      // Use Donut Browser's existing profile launch to open KOL login page
      // The user logs in manually, then we capture cookies
      await invoke("kol_login_kol_platform");
    } catch (e) {
      console.error("Failed to launch KOL login:", e);
    }
  };

  const handleAddDouyin = async () => {
    try {
      await invoke("kol_login_douyin");
    } catch (e) {
      console.error("Failed to launch DouYin login:", e);
    }
  };

  const handleRefreshKol = async (kolId: number) => {
    try {
      await invoke("kol_refresh_kol", { kolId });
    } catch (e) {
      console.error("Failed to refresh KOL:", e);
    }
  };

  const handleRefreshDouyin = async (douyinId: number) => {
    try {
      await invoke("kol_refresh_douyin", { douyinId });
    } catch (e) {
      console.error("Failed to refresh DouYin:", e);
    }
  };

  return (
    <div className="space-y-4">
      <Tabs value={tab} onValueChange={setTab}>
        <div className="flex items-center justify-between">
          <TabsList>
            <TabsTrigger value="kol">KOL 账号 ({kolAccounts.length})</TabsTrigger>
            <TabsTrigger value="douyin">抖音账号 ({douyinAccounts.length})</TabsTrigger>
          </TabsList>
          <div className="flex gap-2">
            {tab === "kol" && (
              <Button size="sm" onClick={handleAddKol}>
                添加 KOL
              </Button>
            )}
            {tab === "douyin" && (
              <Button size="sm" onClick={handleAddDouyin}>
                添加抖音账号
              </Button>
            )}
            <Button size="sm" variant="outline" onClick={refreshAll} disabled={loading}>
              刷新
            </Button>
          </div>
        </div>

        <TabsContent value="kol" className="mt-4">
          <Card>
            <CardContent className="p-0">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead className="w-16">ID</TableHead>
                    <TableHead>UID</TableHead>
                    <TableHead>身份名称</TableHead>
                    <TableHead>备注</TableHead>
                    <TableHead className="w-20">状态</TableHead>
                    <TableHead>创建时间</TableHead>
                    <TableHead className="w-32">操作</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {kolAccounts.length === 0 ? (
                    <TableRow>
                      <TableCell colSpan={7} className="text-center text-muted-foreground">
                        暂无 KOL 账号
                      </TableCell>
                    </TableRow>
                  ) : (
                    kolAccounts.map((kol) => (
                      <TableRow key={kol.id}>
                        <TableCell className="font-mono text-xs">{kol.id}</TableCell>
                        <TableCell className="text-xs">{kol.uid || "-"}</TableCell>
                        <TableCell>{kol.identity_name || "-"}</TableCell>
                        <TableCell className="text-xs text-muted-foreground">
                          {kol.remark || "-"}
                        </TableCell>
                        <TableCell>
                          <Badge variant={kol.status === 1 ? "default" : "secondary"}>
                            {kol.status === 1 ? "正常" : "停用"}
                          </Badge>
                        </TableCell>
                        <TableCell className="text-xs">
                          {new Date(kol.created_at).toLocaleDateString()}
                        </TableCell>
                        <TableCell>
                          <div className="flex gap-1">
                            <Button
                              size="sm"
                              variant="ghost"
                              onClick={() => handleRefreshKol(kol.id)}
                            >
                              刷新
                            </Button>
                            <Button
                              size="sm"
                              variant="ghost"
                              className="text-destructive"
                              onClick={() => deleteKol(kol.id)}
                            >
                              删除
                            </Button>
                          </div>
                        </TableCell>
                      </TableRow>
                    ))
                  )}
                </TableBody>
              </Table>
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="douyin" className="mt-4">
          <Card>
            <CardContent className="p-0">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead className="w-16">ID</TableHead>
                    <TableHead>昵称</TableHead>
                    <TableHead>备注</TableHead>
                    <TableHead className="w-20">状态</TableHead>
                    <TableHead>创建时间</TableHead>
                    <TableHead className="w-32">操作</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {douyinAccounts.length === 0 ? (
                    <TableRow>
                      <TableCell colSpan={6} className="text-center text-muted-foreground">
                        暂无抖音账号
                      </TableCell>
                    </TableRow>
                  ) : (
                    douyinAccounts.map((dy) => (
                      <TableRow key={dy.id}>
                        <TableCell className="font-mono text-xs">{dy.id}</TableCell>
                        <TableCell>{dy.nickname || "-"}</TableCell>
                        <TableCell className="text-xs text-muted-foreground">
                          {dy.remark || "-"}
                        </TableCell>
                        <TableCell>
                          <Badge variant={dy.status === 0 ? "default" : "secondary"}>
                            {dy.status === 0 ? "可用" : "未登录"}
                          </Badge>
                        </TableCell>
                        <TableCell className="text-xs">
                          {new Date(dy.created_at).toLocaleDateString()}
                        </TableCell>
                        <TableCell>
                          <div className="flex gap-1">
                            <Button
                              size="sm"
                              variant="ghost"
                              onClick={() => handleRefreshDouyin(dy.id)}
                            >
                              刷新
                            </Button>
                            <Button
                              size="sm"
                              variant="ghost"
                              className="text-destructive"
                              onClick={() => deleteDouyin(dy.id)}
                            >
                              删除
                            </Button>
                          </div>
                        </TableCell>
                      </TableRow>
                    ))
                  )}
                </TableBody>
              </Table>
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>
    </div>
  );
}
