/**
 * Tomato KOL Server API Client
 *
 * Handles all communication with the tomato-server backend.
 * Token is stored in localStorage and attached to every request.
 */

import type {
  ApiResponse,
  LoginRequest,
  LoginResponse,
  AccountInfo,
  CreateAccountRequest,
  KolAccount,
  KolAccountBase,
  DouYinAccount,
  DouYinAccountBase,
  QiMaoAccount,
  SubmitBrushTaskRequest,
  TaskDataGrid,
  TaskSummary,
  TaskQueryRequest,
  RecentTaskPoint,
  KolIncome,
  FrequencyPoint,
  KolBook,
  CommonSetting,
  DomConfig,
  IncomeNotice,
  ServerBrowserProfile,
} from "../types";

const TOKEN_KEY = "kol_auth_token";
const SERVER_URL_KEY = "kol_server_url";
const DEFAULT_SERVER_URL = "http://localhost:8099";

class KolApiClient {
  private get baseUrl(): string {
    if (typeof window !== "undefined") {
      return localStorage.getItem(SERVER_URL_KEY) || DEFAULT_SERVER_URL;
    }
    return DEFAULT_SERVER_URL;
  }

  private get token(): string | null {
    if (typeof window !== "undefined") {
      return localStorage.getItem(TOKEN_KEY);
    }
    return null;
  }

  setServerUrl(url: string) {
    localStorage.setItem(SERVER_URL_KEY, url);
  }

  setToken(token: string) {
    localStorage.setItem(TOKEN_KEY, token);
  }

  clearToken() {
    localStorage.removeItem(TOKEN_KEY);
  }

  get isLoggedIn(): boolean {
    return !!this.token;
  }

  private async request<T>(
    method: string,
    path: string,
    body?: unknown,
  ): Promise<ApiResponse<T>> {
    const headers: Record<string, string> = {
      "Content-Type": "application/json",
    };
    if (this.token) {
      headers["Authorization"] = `Bearer ${this.token}`;
    }

    const res = await fetch(`${this.baseUrl}/api/v1${path}`, {
      method,
      headers,
      body: body ? JSON.stringify(body) : undefined,
    });

    if (res.status === 401) {
      this.clearToken();
      throw new Error("Unauthorized - please login again");
    }

    return res.json();
  }

  private get<T>(path: string) {
    return this.request<T>("GET", path);
  }
  private post<T>(path: string, body?: unknown) {
    return this.request<T>("POST", path, body);
  }
  private put<T>(path: string, body?: unknown) {
    return this.request<T>("PUT", path, body);
  }
  private del<T>(path: string) {
    return this.request<T>("DELETE", path);
  }

  // ======================== Auth ========================

  async login(req: LoginRequest): Promise<LoginResponse> {
    const res = await this.post<LoginResponse>("/auth/login", req);
    if (res.success && res.data) {
      this.setToken(res.data.token);
      return res.data;
    }
    throw new Error(res.message || "Login failed");
  }

  async healthCheck(): Promise<boolean> {
    const res = await this.get<unknown>("/auth/test");
    return res.success;
  }

  async getVersion(): Promise<string> {
    const res = await this.get<string>("/auth/version");
    return res.data || "unknown";
  }

  // ======================== Account ========================

  async getAccountInfo(): Promise<AccountInfo> {
    const res = await this.get<AccountInfo>("/account");
    return res.data!;
  }

  async createSubAccount(req: CreateAccountRequest): Promise<{ id: number }> {
    const res = await this.post<{ id: number }>("/account/create", req);
    return res.data!;
  }

  async getSubAccounts(): Promise<AccountInfo[]> {
    const res = await this.get<AccountInfo[]>("/account/subs");
    return res.data || [];
  }

  async renewAccount(id: number): Promise<void> {
    await this.post(`/account/${id}/renew`);
  }

  async disableAccount(id: number): Promise<void> {
    await this.post(`/account/${id}/disable`);
  }

  async enableAccount(id: number): Promise<void> {
    await this.post(`/account/${id}/enable`);
  }

  // ======================== KOL Account ========================

  async submitKolCookies(cookies: string, uid?: string, remark?: string): Promise<{ id: number }> {
    const res = await this.post<{ id: number }>("/kol/cookies", {
      cookies, uid, identity_name: undefined, remark,
    });
    return res.data!;
  }

  async updateKolCookies(id: number, cookies: string): Promise<void> {
    await this.put("/kol/cookies", { id, cookies });
  }

  async getKolAccounts(): Promise<KolAccount[]> {
    const res = await this.get<KolAccount[]>("/kol/list");
    return res.data || [];
  }

  async getKolBaseInfos(): Promise<KolAccountBase[]> {
    const res = await this.get<KolAccountBase[]>("/kol/base");
    return res.data || [];
  }

  async getKolById(id: number): Promise<KolAccount> {
    const res = await this.get<KolAccount>(`/kol/${id}`);
    return res.data!;
  }

  async deleteKolAccount(id: number): Promise<void> {
    await this.del(`/kol/${id}`);
  }

  async updateKolRemark(id: number, remark: string): Promise<void> {
    await this.put(`/kol/${id}/remark`, { remark });
  }

  async getInviteCodes(): Promise<unknown[]> {
    const res = await this.get<unknown[]>("/kol/invitecodes");
    return res.data || [];
  }

  // ======================== DouYin Account ========================

  async submitDouYinStorageState(
    storage_state: string,
    nickname?: string,
    remark?: string,
  ): Promise<{ id: number }> {
    const res = await this.post<{ id: number }>("/douyin/storage", {
      storage_state, nickname, remark,
    });
    return res.data!;
  }

  async updateDouYinStorageState(id: number, storage_state: string): Promise<void> {
    await this.put("/douyin/storage", { id, storage_state });
  }

  async getDouYinAccounts(): Promise<DouYinAccount[]> {
    const res = await this.get<DouYinAccount[]>("/douyin/list");
    return res.data || [];
  }

  async getDouYinBaseAccounts(): Promise<DouYinAccountBase[]> {
    const res = await this.get<DouYinAccountBase[]>("/douyin/base");
    return res.data || [];
  }

  async getDouYinById(id: number): Promise<DouYinAccount> {
    const res = await this.get<DouYinAccount>(`/douyin/${id}`);
    return res.data!;
  }

  async deleteDouYinAccount(id: number): Promise<void> {
    await this.del(`/douyin/${id}`);
  }

  async setDouYinStatus(id: number, status: number): Promise<void> {
    await this.put(`/douyin/${id}/status`, { status });
  }

  async updateDouYinRemark(id: number, remark: string): Promise<void> {
    await this.put(`/douyin/${id}/remark`, { remark });
  }

  // ======================== Submit Brush Task ========================

  async submitBrushTask(req: SubmitBrushTaskRequest): Promise<boolean> {
    const res = await this.post<boolean>("/submit/brush", req);
    return res.data ?? false;
  }

  async getRequestFrequency(interval?: string): Promise<FrequencyPoint[]> {
    const q = interval ? `?interval=${interval}` : "";
    const res = await this.get<FrequencyPoint[]>(`/submit/frequency${q}`);
    return res.data || [];
  }

  // ======================== Task Data ========================

  async getTaskDataGrid(query: TaskQueryRequest): Promise<TaskDataGrid> {
    const res = await this.post<TaskDataGrid>("/task/grid", query);
    return res.data!;
  }

  async getTaskSummary(): Promise<TaskSummary> {
    const res = await this.get<TaskSummary>("/task/summary");
    return res.data!;
  }

  async getRecentTasks(): Promise<RecentTaskPoint[]> {
    const res = await this.get<RecentTaskPoint[]>("/task/recent");
    return res.data || [];
  }

  async getRecentIncome(): Promise<KolIncome[]> {
    const res = await this.get<KolIncome[]>("/task/income");
    return res.data || [];
  }

  async getBooks(platform?: number): Promise<KolBook[]> {
    const q = platform ? `?platform=${platform}` : "";
    const res = await this.get<KolBook[]>(`/task/books${q}`);
    return res.data || [];
  }

  // ======================== Settings ========================

  async getAllSettings(): Promise<CommonSetting[]> {
    const res = await this.get<CommonSetting[]>("/setting/all");
    return res.data || [];
  }

  async savePlatformTypes(kol_id: number, open_types: number[]): Promise<void> {
    await this.post("/setting/platform", { kol_id, open_types });
  }

  async saveTypeLimit(kol_id: number, platform: number, limit: number): Promise<void> {
    await this.post("/setting/limit", { kol_id, platform, limit });
  }

  async getDouYinDom(): Promise<DomConfig> {
    const res = await this.get<DomConfig>("/setting/dom/douyin");
    return res.data || {};
  }

  async getKolDom(): Promise<Record<string, string>> {
    const res = await this.get<Record<string, string>>("/setting/dom/kol");
    return res.data || {};
  }

  async updateDomConfig(dom_type: string, selectors: Record<string, unknown>): Promise<void> {
    await this.post("/setting/dom", { dom_type, selectors });
  }

  async getIncomeNotice(): Promise<IncomeNotice[]> {
    const res = await this.get<IncomeNotice[]>("/setting/notice");
    return res.data || [];
  }

  async setIncomeNotice(emails: string[], has_child: boolean): Promise<void> {
    await this.post("/setting/notice", { emails, has_child });
  }

  async getThirdPartyLimit(): Promise<{ kol_count: number; douyin_count: number }> {
    const res = await this.get<{ kol_count: number; douyin_count: number }>("/setting/authorize/limit");
    return res.data!;
  }

  // ======================== Profile Sync ========================

  async createServerProfile(
    name: string,
    browser_type?: string,
    fingerprint_config?: Record<string, unknown>,
  ): Promise<{ id: string }> {
    const res = await this.post<{ id: string }>("/profile", {
      name, browser_type, fingerprint_config,
    });
    return res.data!;
  }

  async listServerProfiles(): Promise<ServerBrowserProfile[]> {
    const res = await this.get<ServerBrowserProfile[]>("/profile");
    return res.data || [];
  }

  async getServerProfile(id: string): Promise<ServerBrowserProfile> {
    const res = await this.get<ServerBrowserProfile>(`/profile/${id}`);
    return res.data!;
  }

  async deleteServerProfile(id: string): Promise<void> {
    await this.del(`/profile/${id}`);
  }
}

// Singleton export
export const kolApi = new KolApiClient();
