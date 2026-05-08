import {
  type AdminSettings,
  type AdminSettingsUpdate,
  type IncomeOverview,
  type IncomeRow,
  type MyTier2ContributionUpdate,
  type QimaoNoticeRow,
  ApiError,
  type ApiLogDeleteRequest,
  type ApiLogMarkRequest,
  type ApiLogQuery,
  type CreateUserRequest,
  type DouyinVideo,
  type EmailSettings,
  type EmailSettingsUpdate,
  type EmailTestResult,
  type JobRun,
  type JobSummary,
  type ListDouyinVideosQuery,
  type LoginRequest,
  type LoginResponse,
  type PagedApiLog,
  type QimaoBook,
  type QimaoBooksRefreshResult,
  type QimaoStatsAccount,
  type QimaoStatsOverview,
  type QimaoTokenResult,
  type TomatoBook,
  type TomatoBooksRefreshResult,
  type TomatoStatsAccount,
  type TomatoStatsOverview,
  type UpdateUserRequest,
  type User,
} from "../types";

const TOKEN_KEY = "kol_token";
// Hard-coded production server. Not user-configurable — the desktop
// client always talks to this single backend.
const SERVER_URL = "http://211.101.236.27:8099";

// `typeof localStorage` may be "object" in some SSR/Edge contexts while the
// actual value is a stub missing `.getItem` — check `window` to be safe.
function safeStorage(): Storage | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage ?? null;
  } catch {
    return null;
  }
}

class KolApi {
  private readonly serverUrl: string = SERVER_URL;
  private token: string | null;

  constructor() {
    this.token = safeStorage()?.getItem(TOKEN_KEY) ?? null;
  }

  get isLoggedIn(): boolean {
    return this.token !== null;
  }

  getServerUrl(): string {
    return this.serverUrl;
  }

  getToken(): string | null {
    return this.token;
  }

  clearToken(): void {
    this.token = null;
    safeStorage()?.removeItem(TOKEN_KEY);
  }

  private setToken(token: string): void {
    this.token = token;
    safeStorage()?.setItem(TOKEN_KEY, token);
  }

  private async request<T>(
    method: string,
    path: string,
    body?: unknown,
  ): Promise<T> {
    const headers: Record<string, string> = {};
    if (body !== undefined) headers["Content-Type"] = "application/json";
    if (this.token) headers.Authorization = `Bearer ${this.token}`;

    const res = await fetch(`${this.serverUrl}${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });

    if (res.status === 401) {
      this.clearToken();
      throw new ApiError(401, "未登录或登录已过期");
    }
    if (res.status === 204) return undefined as T;

    const text = await res.text();
    const parsed = text ? (JSON.parse(text) as unknown) : null;

    if (!res.ok) {
      const message =
        parsed &&
        typeof parsed === "object" &&
        "error" in parsed &&
        typeof (parsed as { error: unknown }).error === "string"
          ? (parsed as { error: string }).error
          : `HTTP ${res.status}`;
      throw new ApiError(res.status, message);
    }
    return parsed as T;
  }

  async login(req: LoginRequest): Promise<LoginResponse> {
    const res = await this.request<LoginResponse>(
      "POST",
      "/api/auth/login",
      req,
    );
    this.setToken(res.token);
    return res;
  }

  me(): Promise<User> {
    return this.request<User>("GET", "/api/auth/me");
  }

  listUsers(): Promise<User[]> {
    return this.request<User[]>("GET", "/api/admin/users");
  }

  createUser(req: CreateUserRequest): Promise<User> {
    return this.request<User>("POST", "/api/admin/users", req);
  }

  updateUser(id: number, req: UpdateUserRequest): Promise<User> {
    return this.request<User>("PATCH", `/api/admin/users/${id}`, req);
  }

  deleteUser(id: number): Promise<void> {
    return this.request<void>("DELETE", `/api/admin/users/${id}`);
  }

  listJobs(): Promise<JobSummary[]> {
    return this.request<JobSummary[]>("GET", "/api/admin/jobs");
  }

  getJobHistory(jobName: string, limit = 50): Promise<JobRun[]> {
    return this.request<JobRun[]>(
      "GET",
      `/api/admin/jobs/${encodeURIComponent(jobName)}/history?limit=${limit}`,
    );
  }

  listTomatoBooks(): Promise<TomatoBook[]> {
    return this.request<TomatoBook[]>("GET", "/api/tomato/books");
  }

  refreshTomatoBooks(): Promise<TomatoBooksRefreshResult> {
    return this.request<TomatoBooksRefreshResult>(
      "POST",
      "/api/tomato/books/refresh",
    );
  }

  listQimaoBooks(): Promise<QimaoBook[]> {
    return this.request<QimaoBook[]>("GET", "/api/qimao/books");
  }

  refreshQimaoBooks(): Promise<QimaoBooksRefreshResult> {
    return this.request<QimaoBooksRefreshResult>(
      "POST",
      "/api/qimao/books/refresh",
    );
  }

  /// Manually trigger a `/user/signin` call against the profile's
  /// stored credentials and persist the resulting `x-qm-devops-token`.
  /// The server's qimao_token_refresh worker also does this every 12h —
  /// this endpoint is the "I changed my password, refresh now" hatch.
  refreshQimaoToken(profileId: string): Promise<QimaoTokenResult> {
    return this.request<QimaoTokenResult>(
      "POST",
      `/api/profiles/${profileId}/qimao_refresh_token`,
    );
  }

  getEmailSettings(): Promise<EmailSettings> {
    return this.request<EmailSettings>("GET", "/api/admin/email_settings");
  }

  updateEmailSettings(payload: EmailSettingsUpdate): Promise<{ ok: boolean }> {
    return this.request<{ ok: boolean }>(
      "PUT",
      "/api/admin/email_settings",
      payload,
    );
  }

  /// Synchronously sends a one-off test email. `to` overrides the
  /// configured recipients[0]; pass undefined to use the default.
  sendTestEmail(to?: string): Promise<EmailTestResult> {
    return this.request<EmailTestResult>(
      "POST",
      "/api/admin/email_settings/test",
      { to },
    );
  }

  getQimaoStatsOverview(): Promise<QimaoStatsOverview> {
    return this.request<QimaoStatsOverview>(
      "GET",
      "/api/qimao/stats/overview",
    );
  }

  getQimaoStatsAccounts(): Promise<QimaoStatsAccount[]> {
    return this.request<QimaoStatsAccount[]>(
      "GET",
      "/api/qimao/stats/accounts",
    );
  }

  getTomatoStatsOverview(): Promise<TomatoStatsOverview> {
    return this.request<TomatoStatsOverview>(
      "GET",
      "/api/tomato/stats/overview",
    );
  }

  getTomatoStatsAccounts(): Promise<TomatoStatsAccount[]> {
    return this.request<TomatoStatsAccount[]>(
      "GET",
      "/api/tomato/stats/accounts",
    );
  }

  listDouyinVideos(opts?: ListDouyinVideosQuery): Promise<DouyinVideo[]> {
    const qs = new URLSearchParams();
    if (opts?.profileId) qs.set("profile_id", opts.profileId);
    if (opts?.limit !== undefined) qs.set("limit", String(opts.limit));
    const tail = qs.toString() ? `?${qs.toString()}` : "";
    return this.request<DouyinVideo[]>("GET", `/api/douyin/videos${tail}`);
  }

  listApiLog(opts: ApiLogQuery = {}): Promise<PagedApiLog> {
    const qs = new URLSearchParams();
    if (opts.service) qs.set("service", opts.service);
    if (opts.endpoint) qs.set("endpoint", opts.endpoint);
    if (opts.parsed_ok !== undefined) qs.set("parsed_ok", String(opts.parsed_ok));
    if (opts.acknowledged !== undefined)
      qs.set("acknowledged", String(opts.acknowledged));
    if (opts.date_from) qs.set("date_from", opts.date_from);
    if (opts.date_to) qs.set("date_to", opts.date_to);
    if (opts.page !== undefined) qs.set("page", String(opts.page));
    if (opts.page_size !== undefined) qs.set("page_size", String(opts.page_size));
    const tail = qs.toString() ? `?${qs.toString()}` : "";
    return this.request<PagedApiLog>("GET", `/api/admin/api_log${tail}`);
  }

  markApiLog(req: ApiLogMarkRequest): Promise<{ updated: number }> {
    return this.request<{ updated: number }>("POST", "/api/admin/api_log/mark", req);
  }

  deleteApiLog(req: ApiLogDeleteRequest): Promise<{ deleted: number }> {
    return this.request<{ deleted: number }>("DELETE", "/api/admin/api_log", req);
  }

  listProfiles(): Promise<import("@/types").BrowserProfile[]> {
    return this.request("GET", "/api/profiles");
  }

  /// Caller's own tomato/qimao profile configs. Admin sees only their
  /// own profiles too (cross-user edits are gone — admin uses defaults).
  listMyKolConfig(): Promise<import("../types").ProfileConfig[]> {
    return this.request("GET", "/api/users/me/kol_config");
  }

  updateMyKolConfig(
    items: import("../types").KolConfigUpdate[],
  ): Promise<{ updated: number }> {
    return this.request("PUT", "/api/users/me/kol_config", items);
  }

  /// Admin-only: per-(platform, alias_type) defaults used as initial
  /// values when creating new tomato/qimao profiles. Doesn't affect
  /// existing profile configs.
  listKolConfigDefaults(): Promise<import("../types").KolConfigDefault[]> {
    return this.request("GET", "/api/admin/kol_config_defaults");
  }

  updateKolConfigDefaults(
    items: import("../types").KolConfigDefaultUpdate[],
  ): Promise<{ updated: number }> {
    return this.request("PUT", "/api/admin/kol_config_defaults", items);
  }

  /// Singleton global admin settings (currently `contribution_pct`).
  /// Returned `updated_at` lets the UI show "saved 30 seconds ago"
  /// without an extra round trip on save.
  getAdminSettings(): Promise<AdminSettings> {
    return this.request<AdminSettings>("GET", "/api/admin/settings");
  }

  updateAdminSettings(payload: AdminSettingsUpdate): Promise<{ ok: boolean }> {
    return this.request<{ ok: boolean }>("PUT", "/api/admin/settings", payload);
  }

  /// Self-edit the caller's `tier2_contribution_pct` (the rate at which
  /// THEIR own tier-2 subordinates' words flow up to them). This is
  /// distinct from `updateUser` — admin uses that for cross-user edits.
  updateMyTier2Contribution(
    payload: MyTier2ContributionUpdate,
  ): Promise<{ ok: boolean }> {
    return this.request<{ ok: boolean }>(
      "PUT",
      "/api/users/me/tier2_contribution",
      payload,
    );
  }

  /// List the caller's direct tier-2 subordinates. Empty array when
  /// the caller has none. Used by the team settings panel.
  listMySubordinates(): Promise<import("../types").SubordinateRow[]> {
    return this.request<import("../types").SubordinateRow[]>(
      "GET",
      "/api/users/me/subordinates",
    );
  }

  /// Caller changes their own password. Requires old-password
  /// verification (admin uses `updateUser` to reset others' passwords
  /// without verification).
  changeMyPassword(payload: {
    old_password: string;
    new_password: string;
  }): Promise<{ ok: boolean }> {
    return this.request<{ ok: boolean }>(
      "PUT",
      "/api/users/me/password",
      payload,
    );
  }

  /// 整体替换调用者自己的通知邮箱列表。空数组 = 清空(不再接收任何
  /// 通知)。返回服务端规范化后的列表(去重 + trim)。
  updateMyNotifyEmails(payload: {
    notify_emails: string[];
  }): Promise<{ ok: boolean; notify_emails: string[] }> {
    return this.request<{ ok: boolean; notify_emails: string[] }>(
      "PUT",
      "/api/users/me/notify_emails",
      payload,
    );
  }

  /// 番茄收益看板 — caller's own polled tomato accounts with their
  /// latest snapshot, sorted by total_income DESC. Admin sees only
  /// their own profiles too; the all-users view is delivered via the
  /// "[管理员速览]" email digest.
  listIncome(): Promise<IncomeRow[]> {
    return this.request<IncomeRow[]>("GET", "/api/users/me/income");
  }

  getIncomeOverview(): Promise<IncomeOverview> {
    return this.request<IncomeOverview>("GET", "/api/users/me/income/overview");
  }

  /// 七猫收益通知 — caller's own monthly notice history. Same
  /// scoping as listIncome.
  listQimaoNotices(): Promise<QimaoNoticeRow[]> {
    return this.request<QimaoNoticeRow[]>("GET", "/api/users/me/qimao_notices");
  }

  exportApiLog(opts: ApiLogQuery = {}): Promise<{ csv: string; count: number }> {
    const qs = new URLSearchParams();
    if (opts.service) qs.set("service", opts.service);
    if (opts.endpoint) qs.set("endpoint", opts.endpoint);
    if (opts.parsed_ok !== undefined) qs.set("parsed_ok", String(opts.parsed_ok));
    if (opts.acknowledged !== undefined)
      qs.set("acknowledged", String(opts.acknowledged));
    if (opts.date_from) qs.set("date_from", opts.date_from);
    if (opts.date_to) qs.set("date_to", opts.date_to);
    const tail = qs.toString() ? `?${qs.toString()}` : "";
    return this.request<{ csv: string; count: number }>(
      "GET",
      `/api/admin/api_log/export${tail}`,
    );
  }
}

export const kolApi = new KolApi();
