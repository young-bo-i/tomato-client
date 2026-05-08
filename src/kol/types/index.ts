export type Role = "admin" | "user";

export interface User {
  id: number;
  username: string;
  role: Role;
  is_active: boolean;
  /** Notification email — server sends offline alerts here. */
  email: string | null;
  /** 2-level hierarchy. NULL → tier-1 (or admin). Non-null → tier-2,
   * referencing the tier-1 above this user. */
  parent_user_id: number | null;
  /** Username of the parent (filled in by server-side LEFT JOIN).
   * NULL when this row is tier-1. */
  parent_username: string | null;
  /** Rate at which THIS user's tier-2 subordinates' words flow up.
   * Only meaningful for tier-1 with subordinates; admin/tier-2 default to 0. */
  tier2_contribution_pct: number;
  /** True when at least one other row has parent_user_id == this.id.
   * Drives the conditional team-management UI. */
  has_subordinates: boolean;
  created_at: string;
  updated_at: string;
}

export interface LoginRequest {
  username: string;
  password: string;
}

export interface LoginResponse {
  token: string;
  user: User;
}

export interface CreateUserRequest {
  username: string;
  password: string;
  role: Role;
  /** Optional notification email. Receives offline alerts for this
   * user's profiles. */
  email?: string | null;
  /** Optional parent (creates this row as a tier-2 user). Must
   * reference an active tier-1 non-admin user. Admins cannot
   * have a parent. */
  parent_user_id?: number | null;
}

export interface UpdateUserRequest {
  password?: string;
  role?: Role;
  is_active?: boolean;
  /** Tri-state: `undefined` (omit) preserves existing, `null` clears,
   * non-empty string sets. The server applies the same semantics. */
  email?: string | null;
  /** Tri-state: `undefined` preserves, `null` promotes to tier-1,
   * `number` reassigns to a new parent. */
  parent_user_id?: number | null;
  /** Admin-side override of the user's tier2 contribution rate. */
  tier2_contribution_pct?: number;
}

export interface ApiErrorPayload {
  error: string;
}

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

export interface TomatoBookCategory {
  category_id: number;
  category_name: string;
  status?: number;
}

export interface TomatoBook {
  position: number;
  book_id: string;
  book_name: string;
  author: string | null;
  word_num: number | null;
  score: number | null;
  chapter_num: number | null;
  recent_income: number | null;
  thumb_url: string | null;
  book_abstract: string | null;
  categories: TomatoBookCategory[] | null;
  promotion_types: unknown;
  fetched_at: string;
}

export interface TomatoBooksRefreshResult {
  ok: boolean;
  count?: number;
  error?: string;
}

/** One row in the qimao_books snapshot. Field set mirrors the upstream
 * `data.list[]` items the server promotes into columns; `tags` is kept
 * as an opaque JSON array since the upstream's labels are free-form. */
export interface QimaoBook {
  position: number;
  book_id: number;
  book_name: string;
  author: string | null;
  first_category: string | null;
  second_category: string | null;
  /** Display string from the upstream (e.g. "1355.91万字"). */
  words_num_text: string | null;
  /** Raw word count integer. */
  words: number | null;
  cover: string | null;
  intro: string | null;
  income_text: string | null;
  is_forbid: boolean;
  is_rights: boolean;
  ad_status: number | null;
  tags: unknown;
  fetched_at: string;
}

export interface QimaoBooksRefreshResult {
  ok: boolean;
  count?: number;
  error?: string;
}

export interface QimaoTokenResult {
  ok: boolean;
  error?: string;
}

/** Global counters across all qimao_aliases. Mirrors TomatoStatsOverview
 * but adds `awaiting_alias_id` (qimao-specific: rows submitted to the
 * platform but whose alias_id we haven't yet polled out of keyword_page). */
export interface QimaoStatsOverview {
  total: number;
  submit_pending: number;
  submit_done: number;
  submit_failed: number;
  awaiting_alias_id: number;
  backfill_pending: number;
  backfill_done: number;
  backfill_failed: number;
}

/** Read shape of /api/admin/email_settings. Password is intentionally
 * NOT returned — `is_password_set` lets the form show "(已保存)"
 * without exposing the value. */
export interface EmailSettings {
  smtp_host: string;
  smtp_port: number;
  smtp_username: string;
  is_password_set: boolean;
  from_address: string;
  from_name: string;
  use_tls: boolean;
  recipients: string[];
  updated_at: string;
}

/** Write shape. `smtp_password === null` (omit) preserves the existing
 * value; `""` clears it; any other string sets it. */
export interface EmailSettingsUpdate {
  smtp_host: string;
  smtp_port: number;
  smtp_username: string;
  smtp_password?: string | null;
  from_address: string;
  from_name: string;
  use_tls: boolean;
  recipients: string[];
}

export interface EmailTestResult {
  ok: boolean;
  to?: string;
  error?: string;
}

/** Per-profile rollup. Token health here means "do we have a working
 * x-qm-devops-token?" (refreshed by the server every ~12h). */
export interface QimaoStatsAccount {
  profile_id: string;
  profile_name: string;
  qimao_identifier: string | null;
  has_token: boolean;
  qimao_token_refreshed_at: string | null;
  qimao_token_last_error: string | null;
  submit_done: number;
  submit_failed: number;
  backfill_done: number;
  backfill_failed: number;
  last_submitted_at: string | null;
}

/** Global counters for the tomato dashboard top row. */
export interface TomatoStatsOverview {
  total: number;
  submit_pending: number;
  submit_done: number;
  submit_failed: number;
  /** Eligible-for-backfill subset that hasn't been backfilled yet. */
  backfill_pending: number;
  backfill_done: number;
  backfill_failed: number;
}

/** Per-account row. Counts only work *attributed* to this account. */
export interface TomatoStatsAccount {
  profile_id: string;
  profile_name: string;
  is_online: boolean;
  offline_reason: string | null;
  last_offline_at: string | null;
  cookie_updated_at: string;
  submit_done: number;
  submit_failed: number;
  backfill_done: number;
  backfill_failed: number;
  last_submitted_at: string | null;
}

export interface DouyinVideo {
  id: number;
  profile_id: string;
  aweme_id: string;
  title: string | null;
  /** Chain-extracted book name. NULL when no rule matched. */
  title_filtered: string | null;
  suggest_word: string | null;
  /** Chain-extracted suggest keyword. NULL when no rule matched. */
  suggest_word_filtered: string | null;
  share_url: string | null;
  first_frame_url: string | null;
  captured_at: string;
  inserted_at: string;
}

export interface ListDouyinVideosQuery {
  profileId?: string;
  limit?: number;
}

export interface JobSummary {
  job_name: string;
  total_runs: number;
  successful_runs: number;
  failed_runs: number;
  total_items: number;
  avg_duration_ms: number | null;
  last_ran_at: string | null;
  last_success: boolean | null;
  last_error: string | null;
}

export interface JobRun {
  id: number;
  job_name: string;
  ran_at: string;
  duration_ms: number | null;
  items_processed: number;
  success: boolean;
  error_reason: string | null;
}

export interface ApiLogRow {
  id: number;
  service: string;
  endpoint: string;
  request_summary: unknown | null;
  http_status: number | null;
  raw_response: unknown | null;
  parsed_ok: boolean;
  parse_error: string | null;
  acknowledged: boolean;
  acknowledged_at: string | null;
  created_at: string;
}

export interface PagedApiLog {
  rows: ApiLogRow[];
  total: number;
  page: number;
  page_size: number;
}

export interface ApiLogQuery {
  service?: string;
  endpoint?: string;
  parsed_ok?: boolean;
  acknowledged?: boolean;
  date_from?: string;
  date_to?: string;
  page?: number;
  page_size?: number;
}

export interface ApiLogMarkRequest {
  ids: number[];
  acknowledged: boolean;
}

export interface ApiLogDeleteRequest {
  ids: number[];
}

export interface KolConfigRow {
  profile_id: string;
  platform: string;
  alias_type: number;
  enabled: boolean;
  daily_limit: number;
  updated_at: string;
}

export interface ProfileConfig {
  profile_id: string;
  profile_name: string;
  kol_platform: string;
  user_id: number;
  username: string;
  is_admin: boolean;
  configs: KolConfigRow[];
}

export interface KolConfigUpdate {
  profile_id: string;
  platform: string;
  alias_type: number;
  enabled: boolean;
  daily_limit: number;
}

/** Read shape of `/api/admin/settings`. Singleton row of global runtime
 * knobs. Currently exposes `admin_contribution_pct`. */
export interface AdminSettings {
  /** 0..=100. Share of every non-admin user's words that the
   * submission router routes to the admin pool (step 1 of the
   * sequential cascade). 0 = disabled. Only {0,10,20,50,100}
   * accepted at PUT time. */
  admin_contribution_pct: number;
  updated_at: string;
}

/** Write shape. Object wrapper for forward compatibility — more
 * global knobs will likely live here. */
export interface AdminSettingsUpdate {
  admin_contribution_pct: number;
}

/** Self-edit shape for `PUT /api/users/me/tier2_contribution`. */
export interface MyTier2ContributionUpdate {
  tier2_contribution_pct: number;
}

/** One row in `GET /api/users/me/subordinates` — the caller's tier-2
 * subordinates. Used by the team settings panel to show the user
 * exactly who they're configuring contribution rates for. */
export interface SubordinateRow {
  id: number;
  username: string;
  email: string | null;
  is_active: boolean;
  created_at: string;
}

/** One row in the admin income panel. All amounts in 分 (cents). */
export interface IncomeRow {
  profile_id: string;
  profile_name: string;
  owner_user_id: number;
  owner_username: string;
  owner_role: "admin" | "user";

  total_income: number;
  regular_income: number;
  bonus_income: number;
  current_week_income: number;
  current_month_income: number;

  /** Upstream's `latest_update_time` localized. NULL = never computed. */
  latest_update_time: string | null;

  /** Verbatim arrays from upstream — render as-is. */
  weekly_income_list: WeeklyIncomeEntry[] | null;
  monthly_income_list: MonthlyIncomeEntry[] | null;
  task_income_list: TaskIncomeEntry[] | null;

  /** Most-recent positive jump observed by the poller. */
  last_diff: number;
  last_diff_at: string | null;
  /** When the diff email for `last_diff_at` was successfully sent.
   * `last_emailed_at < last_diff_at` → email pending (probably failed). */
  last_emailed_at: string | null;
  /** Most recent SMTP failure verbatim. Cleared on success. */
  last_email_error: string | null;
  /** When the poller last refreshed this row (heartbeat). */
  fetched_at: string;
}

export interface WeeklyIncomeEntry {
  start_date: string;
  end_date: string;
  income: number;
  regular_income: number;
  bonus_income: number;
  sharing_income: number;
  invitation_income: number;
  supplement_income: number;
}

export interface MonthlyIncomeEntry {
  month: string;
  income: number;
  regular_income: number;
  bonus_income: number;
  sharing_income: number;
  invitation_income: number;
  supplement_income: number;
}

export interface TaskIncomeEntry {
  task_type: number;
  regular_income: number;
  bonus_income: number;
  total_income: number;
  update_time: string;
  update_time_ts: number;
  supplement_income: number;
}

/** One historical row in the 七猫 monthly income notice panel. */
export interface QimaoNoticeRow {
  profile_id: string;
  profile_name: string;
  owner_user_id: number;
  owner_username: string;

  message_id: number;
  /** "X月KOC七猫免费小说收益明细" or similar. */
  title: string;
  /** Full notice HTML from upstream (inline-styled). UI renders in a
   * sandboxed surface to avoid style bleed. */
  content_html: string;
  /** Upstream's "YYYY-MM-DD" `create_time`, parsed. NULL = unparseable. */
  notice_date: string | null;

  recipient_email: string | null;
  /** NULL when the email send failed; `send_error` carries the SMTP
   * error in that case. */
  emailed_at: string | null;
  send_error: string | null;

  created_at: string;
}

/** Aggregated header for the panel — sum across every polled account. */
export interface IncomeOverview {
  account_count: number;
  total_income: number;
  regular_income: number;
  bonus_income: number;
  current_week_income: number;
  current_month_income: number;
  /** Most recent fetched_at across all rows. */
  last_fetched_at: string | null;
}
