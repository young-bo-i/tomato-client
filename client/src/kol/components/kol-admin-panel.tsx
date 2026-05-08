"use client";

import { useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useKolAdminUsers } from "../hooks/use-kol-admin-users";
import { KolEmailListEditor } from "./kol-email-list-editor";
import type {
  CreateUserRequest,
  Role,
  UpdateUserRequest,
  User,
} from "../types";

interface Props {
  currentUserId: number;
}

export function KolAdminPanel({ currentUserId }: Props) {
  const { users, loading, error, refresh, create, update, remove } =
    useKolAdminUsers();
  const [createOpen, setCreateOpen] = useState(false);
  const [editing, setEditing] = useState<User | null>(null);
  const [deleting, setDeleting] = useState<User | null>(null);

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div className="min-w-0">
          <h2 className="text-lg font-semibold">用户管理</h2>
          <p className="text-xs text-muted-foreground">
            管理员可以创建、禁用和删除账号。注册需由管理员在此页面完成。
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2 shrink-0">
          <Button variant="outline" size="sm" onClick={() => void refresh()}>
            刷新
          </Button>
          <Button size="sm" onClick={() => setCreateOpen(true)}>
            新建用户
          </Button>
        </div>
      </div>

      {error && (
        <div className="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      )}

      <div className="rounded-md border overflow-x-auto">
        <Table className="min-w-[960px]">
          <TableHeader>
            <TableRow>
              <TableHead className="w-16">ID</TableHead>
              <TableHead>用户名</TableHead>
              <TableHead className="w-24">角色</TableHead>
              <TableHead className="w-32">层级</TableHead>
              <TableHead className="w-24">下级贡献</TableHead>
              <TableHead className="w-24">状态</TableHead>
              <TableHead>通知邮箱</TableHead>
              <TableHead>创建时间</TableHead>
              <TableHead className="w-44 text-right">操作</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading && users.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={9}
                  className="text-center text-muted-foreground"
                >
                  加载中...
                </TableCell>
              </TableRow>
            ) : users.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={9}
                  className="text-center text-muted-foreground"
                >
                  暂无用户
                </TableCell>
              </TableRow>
            ) : (
              users.map((u) => {
                const isSelf = u.id === currentUserId;
                // tier2_contribution_pct is only meaningful for tier-1
                // users (no parent + role=user). Show value with a
                // muted hint when 0, "—" for admins/tier-2 since the
                // column is just stored noise for them.
                const isTier1 =
                  u.role === "user" && u.parent_user_id === null;
                return (
                  <TableRow key={u.id}>
                    <TableCell className="font-mono text-xs">{u.id}</TableCell>
                    <TableCell className="font-medium">
                      {u.username}
                      {isSelf && (
                        <span className="ml-2 text-xs text-muted-foreground">
                          (你自己)
                        </span>
                      )}
                    </TableCell>
                    <TableCell>
                      <Badge
                        variant={u.role === "admin" ? "default" : "secondary"}
                      >
                        {u.role === "admin" ? "管理员" : "普通"}
                      </Badge>
                    </TableCell>
                    <TableCell>
                      <TierBadge user={u} />
                    </TableCell>
                    <TableCell>
                      {isTier1 ? (
                        <span
                          className={`text-xs font-mono ${
                            u.has_subordinates
                              ? "text-foreground"
                              : "text-muted-foreground"
                          }`}
                          title={
                            u.has_subordinates
                              ? "此用户的二级下级,采集词中(管理员拿走后)按此比例汇入此用户池"
                              : "此用户暂无下级,值仅保存,实际不生效"
                          }
                        >
                          {u.tier2_contribution_pct}%
                        </span>
                      ) : (
                        <span className="text-xs text-muted-foreground">
                          —
                        </span>
                      )}
                    </TableCell>
                    <TableCell>
                      <Badge variant={u.is_active ? "outline" : "destructive"}>
                        {u.is_active ? "启用" : "停用"}
                      </Badge>
                    </TableCell>
                    <TableCell
                      className="text-xs text-muted-foreground max-w-[260px]"
                      title={u.notify_emails.join("\n")}
                    >
                      {u.notify_emails.length === 0 ? (
                        "—"
                      ) : u.notify_emails.length === 1 ? (
                        <span className="truncate inline-block max-w-full align-middle">
                          {u.notify_emails[0]}
                        </span>
                      ) : (
                        <span>
                          <span className="truncate inline-block max-w-[180px] align-middle">
                            {u.notify_emails[0]}
                          </span>
                          <span className="ml-1 text-[10px] text-muted-foreground">
                            +{u.notify_emails.length - 1}
                          </span>
                        </span>
                      )}
                    </TableCell>
                    <TableCell className="text-xs text-muted-foreground">
                      {new Date(u.created_at).toLocaleString()}
                    </TableCell>
                    <TableCell className="text-right space-x-2">
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={() => setEditing(u)}
                      >
                        编辑
                      </Button>
                      <Button
                        size="sm"
                        variant="destructive"
                        disabled={isSelf}
                        onClick={() => setDeleting(u)}
                      >
                        删除
                      </Button>
                    </TableCell>
                  </TableRow>
                );
              })
            )}
          </TableBody>
        </Table>
      </div>

      <CreateUserDialog
        open={createOpen}
        onOpenChange={setCreateOpen}
        onSubmit={create}
        tier1Candidates={users.filter(
          (u) =>
            u.role === "user" &&
            u.parent_user_id === null &&
            u.is_active,
        )}
      />
      <EditUserDialog
        user={editing}
        currentUserId={currentUserId}
        tier1Candidates={users.filter(
          (u) =>
            u.role === "user" &&
            u.parent_user_id === null &&
            u.is_active &&
            u.id !== editing?.id,
        )}
        subordinates={
          editing
            ? users.filter((u) => u.parent_user_id === editing.id)
            : []
        }
        onOpenChange={(open) => {
          if (!open) setEditing(null);
        }}
        onSubmit={async (patch) => {
          if (editing) await update(editing.id, patch);
        }}
      />
      <DeleteUserDialog
        user={deleting}
        subordinates={
          deleting
            ? users.filter((u) => u.parent_user_id === deleting.id)
            : []
        }
        onOpenChange={(open) => {
          if (!open) setDeleting(null);
        }}
        onConfirm={async () => {
          if (deleting) await remove(deleting.id);
        }}
      />
    </div>
  );
}

/** Sentinel string for the Select-component "no parent" option. The
 * Radix-Select primitive disallows empty-string values, so we use a
 * placeholder and translate to `null` when building the API payload. */
const NO_PARENT = "__none";

function TierBadge({ user }: { user: User }) {
  if (user.role === "admin") {
    return (
      <Badge variant="default" className="text-[10px]">
        管理员
      </Badge>
    );
  }
  if (user.parent_user_id !== null) {
    return (
      <Badge
        variant="secondary"
        className="text-[10px]"
        title={`上级: ${user.parent_username ?? user.parent_user_id}`}
      >
        二级 · {user.parent_username ?? "?"}
      </Badge>
    );
  }
  return (
    <Badge variant="outline" className="text-[10px]">
      一级{user.has_subordinates ? " · 带下级" : ""}
    </Badge>
  );
}

function CreateUserDialog({
  open,
  onOpenChange,
  onSubmit,
  tier1Candidates,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (req: CreateUserRequest) => Promise<unknown>;
  /** Tier-1 user candidates (active, role=user, no parent). The
   * server applies the same constraint when validating; surfacing
   * a short-list here avoids round-trip errors. */
  tier1Candidates: User[];
}) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [role, setRole] = useState<Role>("user");
  const [notifyEmails, setNotifyEmails] = useState<string[]>([]);
  const [parentId, setParentId] = useState<string>(NO_PARENT);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // If role flips to admin, force-clear the parent (admin ⊥ tier
  // hierarchy — server would reject anyway). Done in onValueChange so
  // the UI doesn't get into an invalid intermediate state.
  const onRoleChange = (v: Role) => {
    setRole(v);
    if (v === "admin") setParentId(NO_PARENT);
  };

  const handleSubmit = async () => {
    if (!username.trim() || password.length < 6) {
      setError("用户名必填，密码至少 6 位");
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      const parent_user_id =
        role === "user" && parentId !== NO_PARENT ? Number(parentId) : null;
      await onSubmit({
        username: username.trim(),
        password,
        role,
        notify_emails: notifyEmails,
        parent_user_id,
      });
      setUsername("");
      setPassword("");
      setRole("user");
      setNotifyEmails([]);
      setParentId(NO_PARENT);
      onOpenChange(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : "创建失败");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-sm">
        <DialogHeader>
          <DialogTitle>新建用户</DialogTitle>
        </DialogHeader>
        <div className="space-y-4">
          <div className="space-y-2">
            <Label>用户名</Label>
            <Input
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              placeholder="唯一标识"
              autoFocus
            />
          </div>
          <div className="space-y-2">
            <Label>密码</Label>
            <Input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder="至少 6 位"
            />
          </div>
          <div className="space-y-2">
            <Label>角色</Label>
            <Select value={role} onValueChange={(v) => onRoleChange(v as Role)}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="user">普通用户</SelectItem>
                <SelectItem value="admin">管理员</SelectItem>
              </SelectContent>
            </Select>
          </div>
          {role === "user" && (
            <div className="space-y-2">
              <Label>上级用户(可选)</Label>
              <Select value={parentId} onValueChange={setParentId}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={NO_PARENT}>无 — 一级用户</SelectItem>
                  {tier1Candidates.map((u) => (
                    <SelectItem key={u.id} value={String(u.id)}>
                      {u.username}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <p className="text-[11px] text-muted-foreground">
                指定上级后,新用户为「二级用户」,采集词会按上级配置的比例汇入上级池。
              </p>
            </div>
          )}
          <div className="space-y-2">
            <Label>通知邮箱(可选,可填多个)</Label>
            <KolEmailListEditor
              value={notifyEmails}
              onChange={setNotifyEmails}
              disabled={submitting}
            />
          </div>
          {error && <p className="text-sm text-destructive">{error}</p>}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            取消
          </Button>
          <Button onClick={handleSubmit} disabled={submitting}>
            {submitting ? "创建中..." : "创建"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/// Allowed tier2 contribution buckets — same set as the global admin
/// contribution and the team-settings panel.
const TIER2_BUCKETS: { pct: number; label: string }[] = [
  { pct: 0, label: "0%" },
  { pct: 10, label: "10%" },
  { pct: 20, label: "20%" },
  { pct: 50, label: "50%" },
  { pct: 100, label: "100%" },
];

function EditUserDialog({
  user,
  currentUserId,
  tier1Candidates,
  subordinates,
  onOpenChange,
  onSubmit,
}: {
  user: User | null;
  currentUserId: number;
  tier1Candidates: User[];
  /** Direct tier-2 subordinates of `user`. Used to show context next
   * to the contribution selector ("此用户有 3 个下级:A, B, C") so the
   * admin understands the scope of the setting. */
  subordinates: User[];
  onOpenChange: (open: boolean) => void;
  onSubmit: (patch: UpdateUserRequest) => Promise<unknown>;
}) {
  const [password, setPassword] = useState("");
  const [role, setRole] = useState<Role>("user");
  const [isActive, setIsActive] = useState(true);
  const [notifyEmails, setNotifyEmails] = useState<string[]>([]);
  const [parentId, setParentId] = useState<string>(NO_PARENT);
  const [tier2Pct, setTier2Pct] = useState<number>(0);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const isSelf = user !== null && user.id === currentUserId;

  const reset = () => {
    if (!user) return;
    setPassword("");
    setRole(user.role);
    setIsActive(user.is_active);
    setNotifyEmails(user.notify_emails);
    setParentId(
      user.parent_user_id !== null ? String(user.parent_user_id) : NO_PARENT,
    );
    setTier2Pct(user.tier2_contribution_pct);
    setError(null);
  };

  const handleOpenChange = (open: boolean) => {
    if (open) reset();
    onOpenChange(open);
  };

  // Force-clear parent when role flips to admin (server enforces this
  // too — keeping the UI in sync prevents a confusing 400 on save).
  const onRoleChange = (v: Role) => {
    setRole(v);
    if (v === "admin") setParentId(NO_PARENT);
  };

  const handleSubmit = async () => {
    if (!user) return;
    const patch: UpdateUserRequest = {};
    if (password.length > 0) {
      if (password.length < 6) {
        setError("密码至少 6 位");
        return;
      }
      patch.password = password;
    }
    if (role !== user.role) patch.role = role;
    if (isActive !== user.is_active) patch.is_active = isActive;
    // notify_emails: 整体替换 (server normalizes)。比较 = 长度 + 顺序。
    const emailsChanged =
      notifyEmails.length !== user.notify_emails.length ||
      notifyEmails.some((e, i) => e !== user.notify_emails[i]);
    if (emailsChanged) {
      patch.notify_emails = notifyEmails;
    }
    // parent_user_id tri-state.
    const currentParent =
      user.parent_user_id !== null ? String(user.parent_user_id) : NO_PARENT;
    if (parentId !== currentParent) {
      patch.parent_user_id = parentId === NO_PARENT ? null : Number(parentId);
    }
    if (tier2Pct !== user.tier2_contribution_pct) {
      patch.tier2_contribution_pct = tier2Pct;
    }
    if (Object.keys(patch).length === 0) {
      onOpenChange(false);
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      await onSubmit(patch);
      onOpenChange(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : "更新失败");
    } finally {
      setSubmitting(false);
    }
  };

  // tier2_contribution_pct is only meaningful when the row is a tier-1
  // user (no parent + role=user). For admin or tier-2 the column has no
  // routing effect, so we hide the control (avoids confusing the
  // operator about a setting that does nothing).
  const showTier2Editor =
    user !== null && role === "user" && parentId === NO_PARENT;

  return (
    <Dialog open={user !== null} onOpenChange={handleOpenChange}>
      <DialogContent className="max-w-sm">
        <DialogHeader>
          <DialogTitle>编辑用户 {user?.username}</DialogTitle>
        </DialogHeader>
        <div className="space-y-4">
          <div className="space-y-2">
            <Label>重置密码（留空则不修改）</Label>
            <Input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder="至少 6 位"
            />
          </div>
          <div className="space-y-2">
            <Label>角色</Label>
            <Select
              value={role}
              onValueChange={(v) => onRoleChange(v as Role)}
              disabled={isSelf}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="user">普通用户</SelectItem>
                <SelectItem value="admin">管理员</SelectItem>
              </SelectContent>
            </Select>
            {isSelf && (
              <p className="text-xs text-muted-foreground">
                不能降级自己的权限
              </p>
            )}
          </div>
          {role === "user" && (
            <div className="space-y-2">
              <Label>上级用户</Label>
              <Select
                value={parentId}
                onValueChange={setParentId}
                disabled={user?.has_subordinates}
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={NO_PARENT}>无 — 一级用户</SelectItem>
                  {tier1Candidates.map((u) => (
                    <SelectItem key={u.id} value={String(u.id)}>
                      {u.username}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {user?.has_subordinates && (
                <p className="text-[11px] text-muted-foreground">
                  此用户已有下级,无法降级为二级。先解除下级关系再调整。
                </p>
              )}
            </div>
          )}
          {showTier2Editor && (
            <div className="space-y-2">
              <Label>下级贡献度</Label>
              <div className="grid grid-cols-5 gap-1">
                {TIER2_BUCKETS.map((b) => {
                  const selected = tier2Pct === b.pct;
                  return (
                    <button
                      key={b.pct}
                      type="button"
                      onClick={() => setTier2Pct(b.pct)}
                      className={`text-xs font-mono py-1 rounded-md border transition-colors ${
                        selected
                          ? "border-primary bg-primary/10 text-primary"
                          : "border-border bg-card text-muted-foreground hover:bg-muted"
                      }`}
                    >
                      {b.label}
                    </button>
                  );
                })}
              </div>
              {subordinates.length > 0 ? (
                <p
                  className="text-[11px] text-muted-foreground"
                  title={subordinates.map((s) => s.username).join(", ")}
                >
                  此用户有 {subordinates.length} 个下级:
                  {" "}
                  <span className="text-foreground">
                    {subordinates
                      .slice(0, 3)
                      .map((s) => s.username)
                      .join("、")}
                    {subordinates.length > 3 &&
                      ` 等 ${subordinates.length} 人`}
                  </span>
                  。采集词中(管理员拿走后)按此比例汇入此用户池。
                </p>
              ) : (
                <p className="text-[11px] text-muted-foreground">
                  此用户暂无下级,值会保存,但分配新下级后才生效。
                </p>
              )}
            </div>
          )}
          <div className="flex items-center gap-2">
            <Checkbox
              id="is-active"
              checked={isActive}
              onCheckedChange={(v) => setIsActive(v === true)}
              disabled={isSelf}
            />
            <Label htmlFor="is-active" className="cursor-pointer">
              启用账号（停用后无法登录）
            </Label>
          </div>
          <div className="space-y-2">
            <Label>通知邮箱(可填多个)</Label>
            <KolEmailListEditor
              value={notifyEmails}
              onChange={setNotifyEmails}
              disabled={submitting}
            />
          </div>
          {error && <p className="text-sm text-destructive">{error}</p>}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            取消
          </Button>
          <Button onClick={handleSubmit} disabled={submitting}>
            {submitting ? "保存中..." : "保存"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function DeleteUserDialog({
  user,
  subordinates,
  onOpenChange,
  onConfirm,
}: {
  user: User | null;
  /** Direct tier-2 subordinates of `user` (empty if any other case).
   * When non-empty, the deletion will leave them as orphan tier-1 via
   * the FK's `ON DELETE SET NULL`. We surface this so the operator
   * isn't surprised. */
  subordinates: User[];
  onOpenChange: (open: boolean) => void;
  onConfirm: () => Promise<unknown>;
}) {
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleConfirm = async () => {
    setSubmitting(true);
    setError(null);
    try {
      await onConfirm();
      onOpenChange(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : "删除失败");
    } finally {
      setSubmitting(false);
    }
  };

  const hasSubs = subordinates.length > 0;

  return (
    <Dialog open={user !== null} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-sm">
        <DialogHeader>
          <DialogTitle>删除用户</DialogTitle>
        </DialogHeader>
        <div className="space-y-3">
          <p className="text-sm">
            确定要删除用户{" "}
            <span className="font-medium">{user?.username}</span> 吗？ 此操作不可撤销。
          </p>
          {hasSubs && (
            <div className="rounded-md border border-warning/50 bg-warning/10 px-3 py-2 text-xs space-y-1">
              <p className="font-semibold text-warning">
                ⚠ 此用户有 {subordinates.length} 个下级
              </p>
              <p className="text-muted-foreground">
                删除后,以下下级会自动「升级为一级」(他们仍然存在,但脱离层级关系,
                需要你手动重新指派上级):
              </p>
              <p className="text-foreground font-mono break-all">
                {subordinates.map((s) => s.username).join("、")}
              </p>
            </div>
          )}
        </div>
        {error && <p className="text-sm text-destructive">{error}</p>}
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            取消
          </Button>
          <Button
            variant="destructive"
            onClick={handleConfirm}
            disabled={submitting}
          >
            {submitting ? "删除中..." : "确认删除"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
