-- Account system
CREATE TABLE IF NOT EXISTS account (
    id SERIAL PRIMARY KEY,
    account_name VARCHAR(100) NOT NULL UNIQUE,
    password_hash VARCHAR(255) NOT NULL,
    phone VARCHAR(20),
    email VARCHAR(100),
    status SMALLINT NOT NULL DEFAULT 1,
    parent_id INT REFERENCES account(id),
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_account_parent ON account(parent_id) WHERE is_deleted = FALSE;

-- KOL accounts (Tomato platform)
CREATE TABLE IF NOT EXISTS kol_account (
    id SERIAL PRIMARY KEY,
    account_id INT NOT NULL REFERENCES account(id),
    cookies TEXT,
    uid VARCHAR(100),
    identity_name VARCHAR(100),
    identity_number VARCHAR(100),
    payment_account VARCHAR(100),
    mobile VARCHAR(20),
    remark TEXT,
    status SMALLINT NOT NULL DEFAULT 1,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_kol_account_account ON kol_account(account_id) WHERE is_deleted = FALSE;

-- DouYin accounts
CREATE TABLE IF NOT EXISTS douyin_account (
    id SERIAL PRIMARY KEY,
    account_id INT NOT NULL REFERENCES account(id),
    storage_state TEXT,
    nickname VARCHAR(100),
    remark TEXT,
    status SMALLINT NOT NULL DEFAULT 1,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_douyin_account_account ON douyin_account(account_id) WHERE is_deleted = FALSE;

-- Tomato brush tasks
CREATE TABLE IF NOT EXISTS kol_brush_task (
    id BIGSERIAL PRIMARY KEY,
    account_id INT NOT NULL,
    kol_id INT NOT NULL,
    alias_name VARCHAR(200) NOT NULL,
    alias_id VARCHAR(100),
    share_url TEXT,
    first_picture_url TEXT,
    platform SMALLINT NOT NULL,
    task_status SMALLINT NOT NULL DEFAULT 0,
    write_back_status SMALLINT NOT NULL DEFAULT 0,
    write_back_time TIMESTAMP,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_brush_task_account_created ON kol_brush_task(account_id, created_at DESC);
CREATE INDEX idx_brush_task_writeback ON kol_brush_task(write_back_status, created_at DESC) WHERE is_deleted = FALSE;
CREATE INDEX idx_brush_task_alias ON kol_brush_task(alias_name, created_at DESC);
CREATE INDEX idx_brush_task_kol ON kol_brush_task(kol_id, created_at DESC);

-- Tomato non-task (failed submissions)
CREATE TABLE IF NOT EXISTS kol_brush_non_task (
    id BIGSERIAL PRIMARY KEY,
    account_id INT NOT NULL,
    kol_id INT NOT NULL,
    alias_name VARCHAR(200) NOT NULL,
    share_url TEXT,
    platform SMALLINT NOT NULL,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- QiMao brush tasks
CREATE TABLE IF NOT EXISTS qimao_brush_task (
    id BIGSERIAL PRIMARY KEY,
    account_id INT NOT NULL,
    qimao_account_id INT NOT NULL,
    alias_name VARCHAR(200) NOT NULL,
    alias_id VARCHAR(100),
    share_url TEXT,
    platform SMALLINT NOT NULL DEFAULT 1,
    task_status SMALLINT NOT NULL DEFAULT 0,
    write_back_status SMALLINT NOT NULL DEFAULT 0,
    write_back_time TIMESTAMP,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_qimao_task_writeback ON qimao_brush_task(write_back_status, created_at DESC) WHERE is_deleted = FALSE;
CREATE INDEX idx_qimao_task_alias ON qimao_brush_task(alias_name, created_at DESC);

-- QiMao non-task
CREATE TABLE IF NOT EXISTS qimao_brush_non_task (
    id BIGSERIAL PRIMARY KEY,
    account_id INT NOT NULL,
    qimao_account_id INT NOT NULL,
    alias_name VARCHAR(200) NOT NULL,
    share_url TEXT,
    platform SMALLINT NOT NULL DEFAULT 1,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- QiMao accounts
CREATE TABLE IF NOT EXISTS qimao_account (
    id SERIAL PRIMARY KEY,
    account_id INT NOT NULL REFERENCES account(id),
    phone VARCHAR(20),
    password_hash VARCHAR(255),
    token VARCHAR(500),
    user_info JSONB,
    status SMALLINT NOT NULL DEFAULT 1,
    last_refresh_time TIMESTAMP,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Books (Tomato)
CREATE TABLE IF NOT EXISTS kol_book (
    id BIGSERIAL PRIMARY KEY,
    book_id VARCHAR(100) NOT NULL,
    book_name VARCHAR(500) NOT NULL,
    platform SMALLINT NOT NULL,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_kol_book_platform ON kol_book(platform) WHERE is_deleted = FALSE;

-- Books (QiMao)
CREATE TABLE IF NOT EXISTS qimao_book (
    id BIGSERIAL PRIMARY KEY,
    book_id VARCHAR(100) NOT NULL,
    book_name VARCHAR(500) NOT NULL,
    is_forbidden BOOLEAN NOT NULL DEFAULT FALSE,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Invite codes
CREATE TABLE IF NOT EXISTS kol_invite_code (
    id BIGSERIAL PRIMARY KEY,
    account_id INT NOT NULL,
    kol_id INT NOT NULL,
    invite_code VARCHAR(200) NOT NULL,
    share_token VARCHAR(500),
    x_kol_token VARCHAR(500),
    last_refresh_time TIMESTAMP,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_invite_code_kol ON kol_invite_code(kol_id) WHERE is_deleted = FALSE;

-- Common settings
CREATE TABLE IF NOT EXISTS common_setting (
    id SERIAL PRIMARY KEY,
    account_id INT NOT NULL,
    kol_id INT NOT NULL,
    scene VARCHAR(100) NOT NULL,
    setting_value TEXT NOT NULL,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_setting_account_scene ON common_setting(account_id, scene) WHERE is_deleted = FALSE;

-- DOM configuration
CREATE TABLE IF NOT EXISTS dom_config (
    id SERIAL PRIMARY KEY,
    dom_type VARCHAR(50) NOT NULL UNIQUE,
    selectors JSONB NOT NULL,
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- KOL income
CREATE TABLE IF NOT EXISTS kol_income (
    id BIGSERIAL PRIMARY KEY,
    account_id INT NOT NULL,
    kol_id INT NOT NULL,
    total_income BIGINT NOT NULL DEFAULT 0,
    regular_income BIGINT NOT NULL DEFAULT 0,
    bonus_income BIGINT NOT NULL DEFAULT 0,
    current_month_income BIGINT NOT NULL DEFAULT 0,
    current_week_income BIGINT NOT NULL DEFAULT 0,
    income_json TEXT,
    monthly_income_list_json TEXT,
    weekly_income_list_json TEXT,
    last_update_time TIMESTAMP NOT NULL DEFAULT NOW(),
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_income_kol ON kol_income(kol_id);

-- Income notification settings
CREATE TABLE IF NOT EXISTS income_notice (
    id SERIAL PRIMARY KEY,
    account_id INT NOT NULL,
    email VARCHAR(200) NOT NULL,
    has_child BOOLEAN NOT NULL DEFAULT FALSE,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Submit request tracking (high-frequency writes)
CREATE TABLE IF NOT EXISTS submit_brush_request (
    id BIGSERIAL PRIMARY KEY,
    account_id INT NOT NULL,
    douyin_id INT NOT NULL,
    submit_time TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_submit_request_time ON submit_brush_request(submit_time DESC);
CREATE INDEX idx_submit_request_account ON submit_brush_request(account_id, submit_time DESC);

-- Submit word statistics
CREATE TABLE IF NOT EXISTS submit_word_statistics (
    id BIGSERIAL PRIMARY KEY,
    account_id INT NOT NULL,
    douyin_id INT NOT NULL,
    original_word VARCHAR(500) NOT NULL,
    filter_word VARCHAR(200) NOT NULL,
    submit_time TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Browser profiles (for fingerprint browser sync)
CREATE TABLE IF NOT EXISTS browser_profile (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id INT NOT NULL REFERENCES account(id),
    name VARCHAR(200) NOT NULL,
    browser_type VARCHAR(50) NOT NULL DEFAULT 'chromium',
    fingerprint_config JSONB NOT NULL DEFAULT '{}',
    proxy_config JSONB,
    metadata JSONB,
    last_sync_at TIMESTAMP,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_profile_account ON browser_profile(account_id) WHERE is_deleted = FALSE;

-- Profile archive (tracks synced profile data blobs)
CREATE TABLE IF NOT EXISTS profile_archive (
    id BIGSERIAL PRIMARY KEY,
    profile_id UUID NOT NULL REFERENCES browser_profile(id),
    file_hash VARCHAR(128) NOT NULL,
    file_size BIGINT NOT NULL,
    storage_path VARCHAR(500) NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_archive_profile ON profile_archive(profile_id, created_at DESC);

-- User permissions
CREATE TABLE IF NOT EXISTS permission (
    id SERIAL PRIMARY KEY,
    code VARCHAR(100) NOT NULL UNIQUE,
    name VARCHAR(200) NOT NULL
);

CREATE TABLE IF NOT EXISTS user_auth (
    id SERIAL PRIMARY KEY,
    account_id INT NOT NULL REFERENCES account(id),
    permission_id INT NOT NULL REFERENCES permission(id),
    UNIQUE(account_id, permission_id)
);
