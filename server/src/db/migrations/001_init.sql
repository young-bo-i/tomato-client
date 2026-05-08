-- ============================================================================
-- tomato-kol — Consolidated initial schema
-- ============================================================================
--
-- Single-file consolidated migration. Folds together what used to be a
-- chain of 30 incremental migrations followed by a 6-file rewrite
-- (001 base + 002 admin_settings + 003 user tiers + 004 kol_income +
-- 005 qimao_income_notice + 006 kol_income_email). Every column /
-- constraint / index / trigger / FK that the live application relies
-- on is declared here in one place.
--
-- Deliberate cleanup vs. the legacy incremental history:
--
--   * Removed redundant indexes that duplicate UNIQUE constraints or
--     PK leftmost prefixes:
--       - idx_users_username           (UNIQUE(username) already indexes)
--       - idx_kol_submission_config_profile  (PK leftmost = profile_id)
--       - idx_qimao_aliases_alias_name (UNIQUE(user_id, alias_name))
--       - idx_tomato_aliases_alias_name(UNIQUE(user_id, alias_name, alias_type))
--   * Removed unused indexes:
--       - idx_*_books_book_id          (books cached in-process; no
--                                       per-row WHERE access)
--       - idx_douyin_videos_suggest_word (queried only via the
--                                       *_filtered partial column)
--   * Replaced single-column attribution indexes with covering indexes
--     so admin stats GROUP BY's can satisfy from index without heap
--     reads:
--       - idx_tomato_aliases_submitted_by      → _covering (3-col)
--       - idx_tomato_aliases_backfilled_by     → _covering (2-col)
--       - idx_qimao_aliases_submitted_by_*     newly added covering
--       - idx_qimao_aliases_backfilled_by_*    newly added covering
--   * Added idx_douyin_videos_share_url to fix a 100ms full-table seq
--     scan in backfill_submitter::pick_renewal_link's "is this link
--     still fresh?" EXISTS check (5–10× per minute during renewals).
--
-- Safe to apply only to a FRESH database. To rebuild from an existing
-- DB that was on the old chain:
--   DROP SCHEMA public CASCADE; CREATE SCHEMA public;
-- and let sqlx re-apply this single migration.

-- ----------------------------------------------------------------------------
-- Helper: touch_updated_at trigger function
-- ----------------------------------------------------------------------------
-- Attached to every table with an `updated_at` column so handlers
-- don't have to remember to set it on UPDATE.

CREATE OR REPLACE FUNCTION touch_updated_at() RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- ============================================================================
-- users
-- ============================================================================
-- Application user accounts.
--
-- 2-level hierarchy via `parent_user_id`:
--   * NULL  → tier-1 (default; admin or root user).
--   * non-null → tier-2; FK requires the parent row to exist, the
--     application validator additionally requires the parent to be a
--     tier-1 non-admin (3-level chains rejected at the API layer).
--
-- ON DELETE SET NULL on the self-FK so deleting a tier-1 promotes its
-- tier-2 subordinates back to standalone tier-1 (instead of cascading
-- the delete and silently nuking their data).
--
-- `tier2_contribution_pct` is the rate at which THIS user's tier-2
-- subordinates' words flow up to THIS user (sequential cascade applied
-- after admin_contribution_pct in submission_router). Discrete bucket
-- {0, 10, 20, 50, 100} enforced at the API layer; CHECK is the wider
-- 0..=100 envelope so future bucket changes don't need a migration.
--
-- `email` is the per-user notification destination — NULL means "skip
-- this user in offline-alert dispatch".

CREATE TABLE users (
    id            SERIAL       PRIMARY KEY,
    username      VARCHAR(64)  UNIQUE NOT NULL,
    password_hash VARCHAR(255) NOT NULL,
    role          VARCHAR(16)  NOT NULL DEFAULT 'user'
                  CHECK (role IN ('admin', 'user')),
    is_active     BOOLEAN      NOT NULL DEFAULT TRUE,
    email         TEXT,
    parent_user_id INTEGER     REFERENCES users(id) ON DELETE SET NULL,
    tier2_contribution_pct INTEGER NOT NULL DEFAULT 0
                  CHECK (tier2_contribution_pct BETWEEN 0 AND 100),
    created_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    CONSTRAINT users_parent_user_id_not_self
        CHECK (parent_user_id IS NULL OR parent_user_id <> id)
);

-- (No idx_users_username — UNIQUE constraint already builds an index.)

-- "Find subordinates of user X" lookup; partial because most rows are
-- tier-1 with no parent.
CREATE INDEX idx_users_parent_id
    ON users (parent_user_id)
    WHERE parent_user_id IS NOT NULL;

CREATE TRIGGER users_touch_updated_at
    BEFORE UPDATE ON users
    FOR EACH ROW
    EXECUTE FUNCTION touch_updated_at();

-- ============================================================================
-- browser_profiles
-- ============================================================================
-- Donut Browser profile metadata, plus KOL-specific columns layered on
-- top: kol_platform classification, qimao credentials/token (managed
-- by jobs::qimao_token_refresh), and the douyin login-state mirror
-- (pushed by the Tauri client).
--
-- Runtime-only fields NOT persisted to server: process_id (local PID
-- tracking only).

CREATE TABLE browser_profiles (
    -- Donut baseline
    id                 UUID         PRIMARY KEY,
    user_id            INTEGER      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name               TEXT         NOT NULL,
    browser            TEXT         NOT NULL,
    version            TEXT         NOT NULL,
    release_type       TEXT         NOT NULL DEFAULT 'stable',
    proxy_id           TEXT,
    vpn_id             TEXT,
    group_id           TEXT,
    extension_group_id TEXT,
    tags               JSONB        NOT NULL DEFAULT '[]'::jsonb,
    note               TEXT,
    camoufox_config    JSONB,
    wayfern_config     JSONB,
    sync_mode          TEXT         NOT NULL DEFAULT 'Disabled',
    encryption_salt    TEXT,
    last_sync          BIGINT,
    last_launch        BIGINT,
    host_os            TEXT,
    ephemeral          BOOLEAN      NOT NULL DEFAULT FALSE,
    proxy_bypass_rules JSONB        NOT NULL DEFAULT '[]'::jsonb,
    created_by_id      TEXT,
    created_by_email   TEXT,
    dns_blocklist      TEXT,

    -- KOL classification — 'tomato' / 'qimao' / 'douyin' / NULL.
    -- Deliberately not a CHECK so new platforms don't need a migration.
    kol_platform       TEXT,

    -- 七猫达人 per-profile credentials + server-managed token state.
    -- jobs::qimao_token_refresh signs in every ~12h and persists token.
    qimao_identifier         TEXT,
    qimao_credential         TEXT,
    qimao_token              TEXT,
    qimao_token_refreshed_at TIMESTAMPTZ,
    qimao_token_last_error   TEXT,

    -- Mirror of the Tauri client's douyin login state. Updated via
    -- POST /api/profiles/{id}/douyin_state. Drives offline notifications.
    douyin_login_state            TEXT
        CHECK (douyin_login_state IS NULL
               OR douyin_login_state IN ('authenticated', 'unauthenticated', 'unknown')),
    douyin_login_state_updated_at TIMESTAMPTZ,
    douyin_login_state_url        TEXT,
    -- Set when notification_dispatcher emails about this profile's
    -- offline state; cleared on recovery (state → 'authenticated').
    douyin_offline_notified_at    TIMESTAMPTZ,

    created_at         TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at         TIMESTAMPTZ  NOT NULL DEFAULT NOW(),

    -- Defensive: reject non-array JSON for these fields. `jsonb NOT NULL`
    -- alone allows JSON `null`; this catches malformed POST bodies.
    CONSTRAINT browser_profiles_tags_is_array
        CHECK (jsonb_typeof(tags) = 'array'),
    CONSTRAINT browser_profiles_proxy_bypass_rules_is_array
        CHECK (jsonb_typeof(proxy_bypass_rules) = 'array')
);

CREATE INDEX idx_browser_profiles_user_id ON browser_profiles (user_id);

-- qimao_token_refresh worker scans for stale tokens; partial index
-- stays tiny (only qimao profiles with credentials).
CREATE INDEX idx_browser_profiles_qimao_refresh
    ON browser_profiles (qimao_token_refreshed_at)
    WHERE kol_platform = 'qimao'
      AND qimao_credential IS NOT NULL;

-- notification_dispatcher's "any new douyin offline?" probe — partial
-- index covers only the in-question subset.
CREATE INDEX idx_browser_profiles_douyin_pending_notify
    ON browser_profiles (id)
    WHERE kol_platform = 'douyin'
      AND douyin_login_state = 'unauthenticated'
      AND douyin_offline_notified_at IS NULL;

CREATE TRIGGER browser_profiles_touch_updated_at
    BEFORE UPDATE ON browser_profiles
    FOR EACH ROW
    EXECUTE FUNCTION touch_updated_at();

-- ============================================================================
-- profile_state
-- ============================================================================
-- Per-profile runtime state synced across machines: cookies (structured
-- UnifiedCookie[]), tar.gz of localStorage, and Chromium's per-profile
-- os_crypt_key needed to decrypt cookies on the target machine.

CREATE TABLE profile_state (
    profile_id               UUID         PRIMARY KEY REFERENCES browser_profiles(id) ON DELETE CASCADE,
    cookies                  JSONB,
    cookies_updated_at       TIMESTAMPTZ,
    local_storage            BYTEA,
    local_storage_updated_at TIMESTAMPTZ,
    -- Chromium's per-profile cookie encryption key. Without it the
    -- target machine can't decrypt the synced cookies.
    os_crypt_key             TEXT,
    os_crypt_key_updated_at  TIMESTAMPTZ,
    created_at               TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at               TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE TRIGGER profile_state_touch_updated_at
    BEFORE UPDATE ON profile_state
    FOR EACH ROW
    EXECUTE FUNCTION touch_updated_at();

-- ============================================================================
-- platform_kol_cookies
-- ============================================================================
-- Per-(profile, platform, domain) extracted cookie snapshot used by
-- background workers to talk to platform APIs. is_online flips to
-- FALSE on HTTP 401/403 so workers skip dead cookies until manual
-- re-login. extra_headers carries non-cookie auth (e.g. qimao's
-- x-qm-devops-token sourced from localStorage at extraction time).

CREATE TABLE platform_kol_cookies (
    profile_id          UUID         NOT NULL REFERENCES browser_profiles(id) ON DELETE CASCADE,
    platform            TEXT         NOT NULL,
    domain              TEXT         NOT NULL,
    cookies             JSONB        NOT NULL,
    extra_headers       JSONB        NOT NULL DEFAULT '{}'::jsonb,
    is_online           BOOLEAN      NOT NULL DEFAULT TRUE,
    offline_reason      TEXT,
    last_offline_at     TIMESTAMPTZ,
    -- Set when notification_dispatcher emails for this row's offline
    -- event; cleared on recovery (cookie re-pushed → is_online=TRUE).
    offline_notified_at TIMESTAMPTZ,
    created_at          TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    PRIMARY KEY (profile_id, platform, domain)
);

CREATE INDEX idx_platform_kol_cookies_platform_domain
    ON platform_kol_cookies (platform, domain);

CREATE INDEX idx_platform_kol_cookies_pending_notify
    ON platform_kol_cookies (profile_id)
    WHERE is_online = FALSE AND offline_notified_at IS NULL;

CREATE TRIGGER platform_kol_cookies_touch_updated_at
    BEFORE UPDATE ON platform_kol_cookies
    FOR EACH ROW
    EXECUTE FUNCTION touch_updated_at();

-- ============================================================================
-- tomato_books
-- ============================================================================
-- Daily snapshot of 番茄达人's top ~100 books. tomato_rank truncates
-- and re-inserts daily (no historical retention). Read by the alias
-- enqueue path to randomly assign a book to each filtered keyword.
--
-- Fully cached in-process via services::cache::get_tomato_books, so
-- no per-row WHERE access exists — book_id index would be unused.

CREATE TABLE tomato_books (
    position        INTEGER          PRIMARY KEY,
    book_id         TEXT             NOT NULL,
    book_name       TEXT             NOT NULL,
    author          TEXT,
    word_num        BIGINT,
    score           DOUBLE PRECISION,
    chapter_num     INTEGER,
    recent_income   BIGINT,
    thumb_url       TEXT,
    book_abstract   TEXT,
    categories      JSONB,
    promotion_types JSONB,
    raw             JSONB            NOT NULL,
    fetched_at      TIMESTAMPTZ      NOT NULL DEFAULT NOW()
);

-- ============================================================================
-- qimao_books
-- ============================================================================
-- Daily snapshot of 七猫达人's top ~100 books. Same lifecycle and
-- access pattern as tomato_books.

CREATE TABLE qimao_books (
    position        INTEGER     PRIMARY KEY,
    book_id         BIGINT      NOT NULL,
    book_name       TEXT        NOT NULL,
    author          TEXT,
    first_category  TEXT,
    second_category TEXT,
    words_num_text  TEXT,
    words           BIGINT,
    cover           TEXT,
    intro           TEXT,
    income_text     TEXT,
    is_forbid       BOOLEAN     NOT NULL DEFAULT FALSE,
    is_rights       BOOLEAN     NOT NULL DEFAULT FALSE,
    ad_status       INTEGER,
    tags            JSONB,
    raw             JSONB       NOT NULL,
    fetched_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ============================================================================
-- douyin_videos
-- ============================================================================
-- Video items scraped from Douyin pages by the desktop client. Each
-- (profile, aweme) pair is one row — same video re-seen by the same
-- profile is deduplicated via UNIQUE + ON CONFLICT DO NOTHING.
-- *_filtered columns hold the chain-extracted book name / keyword;
-- NULL means the filter rejected this row's text.

CREATE TABLE douyin_videos (
    id                    BIGSERIAL    PRIMARY KEY,
    profile_id            UUID         NOT NULL REFERENCES browser_profiles(id) ON DELETE CASCADE,
    aweme_id              TEXT         NOT NULL,
    title                 TEXT,
    title_filtered        TEXT,
    suggest_word          TEXT,
    suggest_word_filtered TEXT,
    share_url             TEXT,
    first_frame_url       TEXT,
    captured_at           TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    inserted_at           TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    UNIQUE (profile_id, aweme_id)
);

-- Newest-first list endpoint and per-profile view.
CREATE INDEX idx_douyin_videos_inserted_at
    ON douyin_videos (inserted_at DESC);
CREATE INDEX idx_douyin_videos_profile_inserted
    ON douyin_videos (profile_id, inserted_at DESC);

-- backfill_submitter / qimao_backfill_submitter pick a random Douyin
-- link matching a filtered keyword. Partial indexes on the filtered
-- columns let those branches each hit their own narrow index.
-- (NOTE: idx_douyin_videos_suggest_word on the unfiltered column
--  was removed — the unfiltered text is never queried by value.)
CREATE INDEX idx_douyin_videos_title_filtered
    ON douyin_videos (title_filtered)
    WHERE title_filtered IS NOT NULL;
CREATE INDEX idx_douyin_videos_suggest_word_filtered
    ON douyin_videos (suggest_word_filtered)
    WHERE suggest_word_filtered IS NOT NULL;

-- backfill_submitter::pick_renewal_link's "is this link still fresh?"
-- EXISTS check: WHERE share_url = $1. Without this index it was a
-- full-table seq scan (~100ms on a 700k-row table) running 5–10
-- times/min during renewal cycles. Equality lookup → btree.
CREATE INDEX idx_douyin_videos_share_url
    ON douyin_videos (share_url)
    WHERE share_url IS NOT NULL;

-- ============================================================================
-- external_api_responses
-- ============================================================================
-- Audit trail of every call we make to a third-party API. Lets us
-- diff actual response shapes against parser expectations and replay
-- failures. Successful (parsed_ok=TRUE) calls are sampled at 1/N (env
-- KOL_API_LOG_SAMPLE_OK_RATE, default 10) inside services::api_log to
-- keep table volume in check.

CREATE TABLE external_api_responses (
    id              BIGSERIAL    PRIMARY KEY,
    service         TEXT         NOT NULL,
    endpoint        TEXT         NOT NULL,
    request_summary JSONB,
    -- NULL = transport-layer failure before any HTTP response (DNS,
    -- TLS, signing, etc).
    http_status     INTEGER,
    -- Verbatim body — JSONB if parseable, otherwise wrapped as
    -- {"raw_text": "..."}. Truncated at 8000 chars upstream so a
    -- runaway WAF/CDN HTML response can't bloat one row.
    raw_response    JSONB,
    parsed_ok       BOOLEAN      NOT NULL,
    parse_error     TEXT,
    -- Admin triage flag — "I've seen this error pattern, mute it
    -- from the default unacknowledged view".
    acknowledged    BOOLEAN      NOT NULL DEFAULT FALSE,
    acknowledged_at TIMESTAMPTZ,
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

-- The list endpoint's main filter: service + endpoint + sort by date.
CREATE INDEX idx_external_api_responses_service_endpoint_created
    ON external_api_responses (service, endpoint, created_at DESC);

-- "Show all unparsed responses" admin view.
CREATE INDEX idx_external_api_responses_unparsed
    ON external_api_responses (service, created_at DESC)
    WHERE parsed_ok = FALSE;

-- "Show me 4xx/5xx tonight" ad-hoc query.
CREATE INDEX idx_external_api_responses_http_error
    ON external_api_responses (created_at DESC)
    WHERE http_status IS NOT NULL AND http_status >= 400;

-- Default "needs attention" view (unparsed AND not yet ack'd).
CREATE INDEX idx_external_api_responses_unacked
    ON external_api_responses (service, created_at DESC)
    WHERE acknowledged = FALSE AND parsed_ok = FALSE;

-- ============================================================================
-- tomato_aliases
-- ============================================================================
-- Per-(user, alias_name, alias_type) submission state for words extracted
-- from Douyin and posted to 番茄达人's promotion API. UNIQUE per user
-- means each user's alias namespace is fully isolated.
--
-- Lifecycle (status):
--   pending   → alias_submitter tries promotion/plan/create
--   submitted → got code=0 + alias_id back; backfill cycle starts
--   failed    → terminal; api code != 0 or transport failure
--
-- Lifecycle (backfill_status):
--   pending   → eligible to attach a Douyin post link (after 5min soak)
--   submitted → post/create succeeded; renewal in 29 days
--   failed    → 5 attempts exhausted, or platform terminally rejected
--
-- platform_status mirrors the upstream's review enum (1..6) so we
-- can distinguish "still under review" (transient) from "审核不通过"
-- (terminal) without burning post/create attempts.
--
-- backfill_link_history is an append-only JSONB array — the renewal
-- cycle prefers a fresh link on each 29-day round.
--
-- target_profile_id pins the row to a specific submitting profile
-- (set at enqueue time by submission_router). NULL means "any
-- eligible cookie from the owning user".

CREATE TABLE tomato_aliases (
    id                          BIGSERIAL    PRIMARY KEY,
    user_id                     INTEGER      NOT NULL REFERENCES users(id),
    target_profile_id           UUID         REFERENCES browser_profiles(id),
    book_id                     TEXT         NOT NULL,
    alias_name                  TEXT         NOT NULL,
    alias_type                  INTEGER      NOT NULL CHECK (alias_type IN (1, 2, 6)),
    alias_id                    TEXT,

    -- Submission state
    status                      TEXT         NOT NULL DEFAULT 'pending'
                                CHECK (status IN ('pending', 'submitted', 'failed')),
    error_reason                TEXT,
    submitted_at                TIMESTAMPTZ,
    submitted_by_profile_id     UUID,

    -- Platform review state (refreshed by backfill_submitter when it
    -- polls promotion/plan/list)
    platform_status             INTEGER,
    platform_audit_reason       JSONB,
    platform_status_checked_at  TIMESTAMPTZ,

    -- Backfill state
    backfill_status             TEXT         NOT NULL DEFAULT 'pending'
                                CHECK (backfill_status IN ('pending', 'submitted', 'failed')),
    backfill_attempts           INTEGER      NOT NULL DEFAULT 0,
    backfill_post_link          TEXT,
    backfill_link_history       JSONB        NOT NULL DEFAULT '[]'::jsonb,
    backfill_last_attempt_at    TIMESTAMPTZ,
    backfill_error_reason       TEXT,
    backfilled_at               TIMESTAMPTZ,
    backfilled_by_profile_id    UUID,

    created_at                  TIMESTAMPTZ  NOT NULL DEFAULT NOW(),

    -- Per-user uniqueness: each user submits each (alias_name, alias_type)
    -- at most once. The implicit btree on these columns serves any
    -- (user_id, alias_name) and (user_id) prefix lookup, so no separate
    -- alias_name index is needed.
    CONSTRAINT tomato_aliases_user_alias_type_unique
        UNIQUE (user_id, alias_name, alias_type)
);

-- alias_submitter pending scan
CREATE INDEX idx_tomato_aliases_pending
    ON tomato_aliases (user_id, created_at)
    WHERE status = 'pending';

-- backfill_submitter pending scan
CREATE INDEX idx_tomato_aliases_backfill_pending
    ON tomato_aliases (submitted_at)
    WHERE status = 'submitted' AND backfill_status = 'pending';

-- Dashboard "filter by platform_status" queries
CREATE INDEX idx_tomato_aliases_platform_status
    ON tomato_aliases (platform_status)
    WHERE platform_status IS NOT NULL;

-- handle_renewals 30-day-old submitted-row scan
CREATE INDEX idx_tomato_aliases_submitted_renewal
    ON tomato_aliases (submitted_at)
    WHERE status = 'submitted';

-- backfill_submitter renewal: promote 29-day-old backfilled rows
CREATE INDEX idx_tomato_aliases_backfilled_renewal
    ON tomato_aliases (backfilled_at)
    WHERE backfill_status = 'submitted';

-- target_profile_id-routed pending scan
CREATE INDEX idx_tomato_aliases_target_pending
    ON tomato_aliases (target_profile_id, created_at)
    WHERE status = 'pending' AND target_profile_id IS NOT NULL;

-- Daily-usage GROUP BY for submission_router (cached at the route layer
-- but DB still serves it on cache miss every 5s)
CREATE INDEX idx_tomato_aliases_target_date
    ON tomato_aliases (target_profile_id, alias_type, created_at)
    WHERE target_profile_id IS NOT NULL;

-- Stats accounts page covering indexes — let the per-account GROUP BY
-- aggregations satisfy from index without heap reads. The leftmost
-- column matches the older idx_tomato_aliases_submitted_by /
-- _backfilled_by (which were dropped in the consolidation), so any
-- query they served still goes through here.
CREATE INDEX idx_tomato_aliases_submitted_by_covering
    ON tomato_aliases (submitted_by_profile_id, status, submitted_at)
    WHERE submitted_by_profile_id IS NOT NULL;
CREATE INDEX idx_tomato_aliases_backfilled_by_covering
    ON tomato_aliases (backfilled_by_profile_id, backfill_status)
    WHERE backfilled_by_profile_id IS NOT NULL;

-- ============================================================================
-- qimao_aliases
-- ============================================================================
-- Per-(user, alias_name) submission state for 七猫达人. Differences
-- from tomato_aliases:
--   * Single product (no alias_type fan-out).
--   * alias_id discovered asynchronously via keyword_page polling
--     (qimao's add_keywords doesn't return it).
--   * 30-day age cap: rows still pending after 30 days are
--     terminally failed (mirrors C# QiMaoWriteBackJob.ThrowDay).
--
-- platform_status_code is the upstream's text status:
--   "1" 审核中 / "2" 已通过 / "4" 已发布 / others = invalid.

CREATE TABLE qimao_aliases (
    id                          BIGSERIAL    PRIMARY KEY,
    user_id                     INTEGER      NOT NULL REFERENCES users(id),
    target_profile_id           UUID         REFERENCES browser_profiles(id),
    book_id                     BIGINT       NOT NULL,
    book_name                   TEXT         NOT NULL,
    alias_name                  TEXT         NOT NULL,
    alias_id                    BIGINT,

    -- Submission state
    status                      TEXT         NOT NULL DEFAULT 'pending'
                                CHECK (status IN ('pending', 'submitted', 'failed')),
    error_reason                TEXT,
    submitted_at                TIMESTAMPTZ,
    submitted_by_profile_id     UUID,

    -- Platform review state (refreshed when polling keyword_page)
    platform_status_code        TEXT,
    platform_reject_reason      TEXT,
    platform_status_checked_at  TIMESTAMPTZ,

    -- Backfill state
    backfill_status             TEXT         NOT NULL DEFAULT 'pending'
                                CHECK (backfill_status IN ('pending', 'submitted', 'failed')),
    backfill_attempts           INTEGER      NOT NULL DEFAULT 0,
    backfill_post_link          TEXT,
    backfill_link_history       JSONB        NOT NULL DEFAULT '[]'::jsonb,
    backfill_last_attempt_at    TIMESTAMPTZ,
    backfill_error_reason       TEXT,
    backfilled_at               TIMESTAMPTZ,
    backfilled_by_profile_id    UUID,

    created_at                  TIMESTAMPTZ  NOT NULL DEFAULT NOW(),

    -- Per-user uniqueness on alias_name (qimao has no alias_type axis).
    CONSTRAINT qimao_aliases_user_alias_unique
        UNIQUE (user_id, alias_name)
);

CREATE INDEX idx_qimao_aliases_pending
    ON qimao_aliases (user_id, created_at)
    WHERE status = 'pending';

CREATE INDEX idx_qimao_aliases_backfill_pending
    ON qimao_aliases (user_id, submitted_at)
    WHERE status = 'submitted' AND backfill_status = 'pending';

-- 30-day age-cap UPDATE in qimao_backfill_submitter
CREATE INDEX idx_qimao_aliases_age_gc
    ON qimao_aliases (created_at)
    WHERE status = 'submitted' AND backfill_status = 'pending';

CREATE INDEX idx_qimao_aliases_target_pending
    ON qimao_aliases (target_profile_id, created_at)
    WHERE status = 'pending' AND target_profile_id IS NOT NULL;

CREATE INDEX idx_qimao_aliases_target_date
    ON qimao_aliases (target_profile_id, created_at)
    WHERE target_profile_id IS NOT NULL;

-- Stats accounts covering indexes (same pattern as tomato).
CREATE INDEX idx_qimao_aliases_submitted_by_covering
    ON qimao_aliases (submitted_by_profile_id, status, submitted_at)
    WHERE submitted_by_profile_id IS NOT NULL;
CREATE INDEX idx_qimao_aliases_backfilled_by_covering
    ON qimao_aliases (backfilled_by_profile_id, backfill_status)
    WHERE backfilled_by_profile_id IS NOT NULL;

-- ============================================================================
-- email_settings
-- ============================================================================
-- Singleton table for SMTP / notification email config. id=1 CHECK
-- enforces "at most one settings row" — the API always upserts id=1.
-- Empty defaults so a fresh DB always has a row to read.

CREATE TABLE email_settings (
    id            INTEGER     PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    smtp_host     TEXT        NOT NULL DEFAULT '',
    smtp_port     INTEGER     NOT NULL DEFAULT 587,
    smtp_username TEXT        NOT NULL DEFAULT '',
    smtp_password TEXT        NOT NULL DEFAULT '',
    from_address  TEXT        NOT NULL DEFAULT '',
    from_name     TEXT        NOT NULL DEFAULT '',
    use_tls       BOOLEAN     NOT NULL DEFAULT TRUE,
    recipients    JSONB       NOT NULL DEFAULT '[]'::jsonb,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Seed the singleton row so the API never has to deal with "doesn't
-- exist yet". First GET returns defaults, first PUT updates in place.
INSERT INTO email_settings (id) VALUES (1);

-- ============================================================================
-- admin_settings
-- ============================================================================
-- Singleton row of global runtime knobs configurable by admin via UI.
--
-- `admin_contribution_pct` is the share of every non-admin user's
-- collected words that the submission router redirects into the admin
-- pool (step 1 of the sequential cascade). Distribution is
-- Bresenham-style across each user's per-platform word stream so the
-- long-run ratio matches the configured percentage even for non-clean
-- divisors. Discrete bucket {0, 10, 20, 50, 100} enforced at the API
-- layer; CHECK is the wider 0..=100 envelope.

CREATE TABLE admin_settings (
    id                     INTEGER     PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    admin_contribution_pct INTEGER     NOT NULL DEFAULT 0
                           CHECK (admin_contribution_pct BETWEEN 0 AND 100),
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Seed the singleton.
INSERT INTO admin_settings (id) VALUES (1);

-- ============================================================================
-- job_runs
-- ============================================================================
-- Per-tick execution log for cron jobs and pollers. Admin dashboard
-- reads run counts, durations, and last error from this table.

CREATE TABLE job_runs (
    id              BIGSERIAL    PRIMARY KEY,
    job_name        TEXT         NOT NULL,
    ran_at          TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    duration_ms     INTEGER,
    items_processed INTEGER      NOT NULL DEFAULT 0,
    success         BOOLEAN      NOT NULL DEFAULT TRUE,
    error_reason    TEXT
);

-- Per-job timeline for the "history" view.
CREATE INDEX idx_job_runs_job_name_ran_at
    ON job_runs (job_name, ran_at DESC);
-- Cross-job recency for the summary view.
CREATE INDEX idx_job_runs_ran_at
    ON job_runs (ran_at DESC);

-- ============================================================================
-- kol_submission_config
-- ============================================================================
-- Per-(profile, platform, alias_type) submission throttle. A missing
-- row is treated as enabled=TRUE, daily_limit=0 (unlimited) so new
-- profiles work out-of-the-box before any config is written.
--
-- daily_limit semantics: soft cap. submission_router caches the
-- per-day usage map for 5s; concurrent batches in that window can
-- collectively overshoot by ~10–20%. Set the limit slightly under
-- target if a hard ceiling is required.

CREATE TABLE kol_submission_config (
    profile_id  UUID        NOT NULL REFERENCES browser_profiles(id) ON DELETE CASCADE,
    platform    TEXT        NOT NULL CHECK (platform IN ('tomato', 'qimao')),
    alias_type  INTEGER     NOT NULL,
    enabled     BOOLEAN     NOT NULL DEFAULT TRUE,
    daily_limit INTEGER     NOT NULL DEFAULT 0 CHECK (daily_limit >= 0),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (profile_id, platform, alias_type)
);

-- (No idx_kol_submission_config_profile — the PK's leftmost column
--  is profile_id, so the implicit PK index already serves any
--  WHERE profile_id = X lookup.)

-- ============================================================================
-- kol_income
-- ============================================================================
-- Per-tomato-profile latest "我的收益" (income/stats) snapshot pulled
-- from 番茄达人's `/api/platform/user/income/stats/v:version` endpoint.
-- One row per tomato browser_profile (= one kol-platform account).
--
-- The poller (`jobs::tomato_income`) runs every 10 minutes and obeys
-- two guard-rails ported from the legacy KolScheduled.IncomeNoticeJob:
--
--   1. **2-minute skew gate**: if MAX(latest_update_time) across all
--      accounts in this round is newer than NOW - 2min, skip the
--      round entirely (upstream still settling).
--   2. **Idempotency**: if MAX(upstream LUT) <= MAX(stored LUT), no
--      account has anything newer than what's already persisted —
--      skip the per-row diff + upsert step.
--
-- All amount columns store **整数分** (cents); display layers divide
-- by 100. BIGINT (not INT) so the column never wraps under long-tail
-- earnings; the legacy C# used INT and we'd rather not repeat that
-- capacity ceiling.
--
-- Email tracking columns (last_emailed_at, last_email_error) record
-- the outcome of the per-diff notification email sent by the same
-- poller after a forward jump in total_income.

CREATE TABLE kol_income (
    -- One row per tomato profile. CASCADE so deleting the profile
    -- nukes the orphan income row instead of leaving a dangling FK.
    profile_id              UUID         PRIMARY KEY REFERENCES browser_profiles(id) ON DELETE CASCADE,

    -- Core counters from upstream `data.{total,regular,bonus,
    -- current_week,current_month}_income`. All in 分 (cents).
    total_income            BIGINT       NOT NULL DEFAULT 0,
    regular_income          BIGINT       NOT NULL DEFAULT 0,
    bonus_income            BIGINT       NOT NULL DEFAULT 0,
    current_week_income     BIGINT       NOT NULL DEFAULT 0,
    current_month_income    BIGINT       NOT NULL DEFAULT 0,

    -- Upstream's `latest_update_time` (unix seconds), converted to
    -- TIMESTAMPTZ. The skew + idempotency gates compare against this.
    -- Nullable because a never-fetched row defaults to NULL on first
    -- INSERT before the poller's first successful read.
    latest_update_time      TIMESTAMPTZ,

    -- Verbatim list payloads from upstream — kept JSONB so the UI can
    -- render the historical week/month/per-task breakdown without
    -- another round trip.
    weekly_income_list      JSONB,
    monthly_income_list     JSONB,
    task_income_list        JSONB,

    -- Full upstream `data` object for forensic / future-field-add
    -- without a migration.
    raw                     JSONB,

    -- Most recent positive increment we observed (current total -
    -- previous total at the moment we detected a forward jump). 0
    -- when no diff yet recorded. Drives the admin UI's "🆙 +¥XX"
    -- highlight.
    last_diff               BIGINT       NOT NULL DEFAULT 0,
    last_diff_at            TIMESTAMPTZ,

    -- Email-send tracking.
    --   * last_emailed_at — when the most recent diff email was
    --     SUCCESSFULLY sent. Compared against last_diff_at to detect
    --     "there's a pending diff for which the email failed".
    --   * last_email_error — last SMTP failure verbatim. Cleared on
    --     subsequent success; kept for the admin "✗ 失败" tooltip.
    last_emailed_at         TIMESTAMPTZ,
    last_email_error        TEXT,

    -- When this row was last refreshed by the poller (regardless of
    -- whether the data changed). Heartbeat for "is the income job
    -- alive for this account".
    fetched_at              TIMESTAMPTZ  NOT NULL DEFAULT NOW(),

    created_at              TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at              TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE TRIGGER kol_income_touch_updated_at
    BEFORE UPDATE ON kol_income
    FOR EACH ROW
    EXECUTE FUNCTION touch_updated_at();

-- "Show me accounts with new earnings" — partial on non-null only.
CREATE INDEX idx_kol_income_last_diff_at
    ON kol_income (last_diff_at DESC)
    WHERE last_diff_at IS NOT NULL;

-- Default sort for the admin overview ("top earners overall").
CREATE INDEX idx_kol_income_total_income
    ON kol_income (total_income DESC);

-- ============================================================================
-- qimao_income_notice
-- ============================================================================
-- 七猫达人 doesn't expose a real-time income endpoint like 番茄达人.
-- Instead the platform pushes a monthly "X月KOC七猫免费小说收益明细"
-- notice into the user's site-wide message feed
-- (`/api/v1/message/notice/list`), typically between the 10th and 20th
-- of the following month.
--
-- The qimao_income_notice job polls the message feed for every active
-- qimao profile 3× a day on days 10–20, filters notices whose title
-- contains the income marker, and emails the matching ones to the
-- profile's owner (with email_settings.recipients as fallback). This
-- table is the dedup ledger: PK = (profile_id, message_id). A row's
-- existence is proof we already emailed it; the job's idempotency
-- relies on that.

CREATE TABLE qimao_income_notice (
    profile_id    UUID         NOT NULL REFERENCES browser_profiles(id) ON DELETE CASCADE,
    -- Upstream message id. Globally unique across qimao's feed but
    -- scoped by profile here because each user sees their own feed.
    message_id    BIGINT       NOT NULL,

    -- Verbatim from upstream. `title` is the X月... heading; the UI
    -- shows it as the row label. `content_html` is the full notice
    -- HTML (the upstream sends inline-styled HTML chunks); the email
    -- forwards this as-is, the admin panel renders it in a sandboxed
    -- iframe / dangerouslySetInnerHTML.
    title         TEXT         NOT NULL,
    content_html  TEXT         NOT NULL,

    -- Upstream's `create_time` ("YYYY-MM-DD"), parsed to a date.
    -- Distinct from `emailed_at` (when WE actually sent the email).
    -- Used to sort the admin panel by month.
    notice_date   DATE,

    -- Email pipeline outcome. `recipient_email` is the resolved
    -- destination (owner.email or admin fallback). `emailed_at` is
    -- set on successful SMTP send; left NULL if the send failed
    -- (operator can re-send by deleting the row and waiting for the
    -- next poller fire).
    recipient_email TEXT,
    emailed_at      TIMESTAMPTZ,
    send_error      TEXT,

    created_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    PRIMARY KEY (profile_id, message_id)
);

-- "Latest notices per profile" sort for the admin panel.
CREATE INDEX idx_qimao_income_notice_emailed_at
    ON qimao_income_notice (emailed_at DESC NULLS LAST);

-- Per-profile chronological view.
CREATE INDEX idx_qimao_income_notice_profile_created
    ON qimao_income_notice (profile_id, created_at DESC);
