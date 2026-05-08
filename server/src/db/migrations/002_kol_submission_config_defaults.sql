-- ============================================================================
-- 002_kol_submission_config_defaults — 拆分管理员/用户两层配置
-- ============================================================================
--
-- 之前 `kol_submission_config` 是一张「管理员可改任意 profile」的全集表,
-- 既混了「单 profile 的开关 + 限额」也充当了「新建账号的初始值」。
-- 现在把这两个职责拆开:
--
--   * `kol_submission_config_defaults` — 平台 × alias_type 维度的默认值。
--     只有管理员能改。**只在创建新的 tomato/qimao profile 时被读一次**,
--     用作新增 `kol_submission_config` 行的初始值;之后管理员改这里
--     不会回填已存在的 profile。
--
--   * `kol_submission_config` (已存在,不动) — 单 profile 维度的具体配置。
--     由 profile 的所有者(包括 admin 自己的 profile)自行管理。
--     列出 / 编辑限制在 `users.id = bp.user_id` 范围内。
--
-- 这样:
--   - 管理员页面只看默认值,不会一次性影响所有用户的现有 profile。
--   - 普通用户能管理自己 profile 的配置(以前只有 admin 能改)。

CREATE TABLE kol_submission_config_defaults (
    platform    TEXT        NOT NULL CHECK (platform IN ('tomato', 'qimao')),
    alias_type  INTEGER     NOT NULL,
    enabled     BOOLEAN     NOT NULL DEFAULT TRUE,
    daily_limit INTEGER     NOT NULL DEFAULT 0 CHECK (daily_limit >= 0),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (platform, alias_type)
);

-- 种入合理初始值。tomato 三个 alias_type (1=番茄小说, 2=番茄畅听, 6=悟空浏览器)
-- 与 qimao 单一 alias_type=1 (七猫小说),与代码里 seed_default_submission_config
-- 之前硬编码的列表保持一致。daily_limit=300 是历史默认值。
INSERT INTO kol_submission_config_defaults (platform, alias_type, enabled, daily_limit) VALUES
    ('tomato', 1, TRUE, 300),
    ('tomato', 2, TRUE, 300),
    ('tomato', 6, TRUE, 300),
    ('qimao',  1, TRUE, 300);
