// ============================================================
// KOL Module Types - Maps to tomato-server models
// ============================================================

// --- Auth ---
export interface LoginRequest {
  account: string;
  password: string;
}

export interface LoginResponse {
  token: string;
  account_id: number;
  account_name: string;
}

export interface AccountInfo {
  id: number;
  account_name: string;
  phone?: string;
  email?: string;
  status: number;
  parent_id?: number;
  created_at: string;
}

export interface CreateAccountRequest {
  account_name: string;
  password: string;
  phone?: string;
  email?: string;
}

// --- KOL Account (Tomato/Fanqie platform) ---
export interface KolAccount {
  id: number;
  account_id: number;
  cookies?: string;
  uid?: string;
  identity_name?: string;
  identity_number?: string;
  payment_account?: string;
  mobile?: string;
  remark?: string;
  status: number;
  created_at: string;
}

export interface KolAccountBase {
  id: number;
  account_id: number;
  uid?: string;
  identity_name?: string;
  remark?: string;
  status: number;
  created_at: string;
}

// --- DouYin Account ---
export interface DouYinAccount {
  id: number;
  account_id: number;
  storage_state?: string;
  nickname?: string;
  remark?: string;
  status: number; // 0=Available, 1=NonLogin
  is_deleted: boolean;
  created_at: string;
  updated_at: string;
}

export interface DouYinAccountBase {
  id: number;
  account_id: number;
  nickname?: string;
  remark?: string;
  status: number;
  created_at: string;
}

// --- QiMao Account ---
export interface QiMaoAccount {
  id: number;
  account_id: number;
  phone?: string;
  token?: string;
  user_info?: Record<string, unknown>;
  status: number;
  remark?: string;
  created_at: string;
}

// --- Brush Task ---
export interface BrushTask {
  id: number;
  account_id: number;
  kol_id: number;
  alias_name: string;
  alias_id?: string;
  share_url?: string;
  first_picture_url?: string;
  platform: AliasType;
  task_status: number;
  write_back_status: WriteBackStatus;
  write_back_time?: string;
  created_at: string;
}

export interface SubmitBrushTaskRequest {
  douyin_id: number;
  alias_name: string;
  share_url?: string;
  first_picture_url?: string;
}

export interface TaskDataGrid {
  items: BrushTask[];
  total: number;
  page: number;
  page_size: number;
}

export interface TaskSummary {
  total_count: number;
  today_count: number;
  no_callback_count: number;
}

export interface TaskQueryRequest {
  page?: number;
  page_size?: number;
  date_range?: "day" | "week" | "month";
  platform?: AliasType;
}

// --- Books ---
export interface KolBook {
  id: number;
  book_id: string;
  book_name: string;
  platform: number;
  created_at: string;
}

// --- Settings ---
export interface CommonSetting {
  id: number;
  account_id: number;
  kol_id: number;
  scene: string;
  setting_value: string;
  created_at: string;
}

export interface DomConfig {
  // DouYin DOM selectors (fetched from server, used for automation)
  IsOpenSite?: string;
  IsLogin?: string;
  VideoContainerSelector?: string;
  LiveSelector?: string;
  VideoIdAttr?: string;
  SuggestWork?: string;
  BottomInfo?: string;
  VideoTitle?: string;
  FirstFrame?: string;
  NextButton?: string;
}

export interface KolDomConfig {
  selectors: Record<string, string>;
}

// --- Income ---
export interface KolIncome {
  id: number;
  account_id: number;
  kol_id: number;
  total_income: number;
  regular_income: number;
  bonus_income: number;
  current_month_income: number;
  current_week_income: number;
  last_update_time: string;
}

export interface IncomeNotice {
  id: number;
  account_id: number;
  email: string;
  has_child: boolean;
}

// --- Browser Profile (sync with server) ---
export interface ServerBrowserProfile {
  id: string;
  account_id: number;
  name: string;
  browser_type: string;
  fingerprint_config: Record<string, unknown>;
  proxy_config?: Record<string, unknown>;
  metadata?: Record<string, unknown>;
  last_sync_at?: string;
  created_at: string;
}

// --- Request Frequency ---
export interface FrequencyPoint {
  time_bucket: string;
  count: number;
}

// --- Recent Task Data ---
export interface RecentTaskPoint {
  platform: number;
  day: string;
  count: number;
}

// --- Enums ---
export enum AliasType {
  XiaoShuo = 1,
  TouTiao = 2,
  ChangTing = 3,
  WuKong = 4,
}

export const AliasTypeLabel: Record<AliasType, string> = {
  [AliasType.XiaoShuo]: "小说",
  [AliasType.TouTiao]: "头条",
  [AliasType.ChangTing]: "畅听",
  [AliasType.WuKong]: "悟空",
};

export enum WriteBackStatus {
  Pending = 0,
  Done = 1,
  Expired = 2,
  ChangeUrl = 3,
}

export const WriteBackStatusLabel: Record<WriteBackStatus, string> = {
  [WriteBackStatus.Pending]: "待回填",
  [WriteBackStatus.Done]: "已回填",
  [WriteBackStatus.Expired]: "已过期",
  [WriteBackStatus.ChangeUrl]: "待更换",
};

export enum DouYinAccountStatus {
  Available = 0,
  NonLogin = 1,
}

// --- Auto Gather Config ---
export interface AutoGatherConfig {
  enabled_douyin_ids: number[];
  start_time: string; // "HH:mm"
  end_time: string;   // "HH:mm"
  interval_ms: number; // delay between videos (default 800)
  videos_per_session: number; // default 40
}

// --- API Response Wrapper ---
export interface ApiResponse<T = unknown> {
  success: boolean;
  data?: T;
  message?: string;
}

// --- Gather Log ---
export interface GatherLog {
  id: string;
  timestamp: string;
  douyin_id: number;
  douyin_nickname: string;
  level: "info" | "warn" | "error";
  message: string;
}
