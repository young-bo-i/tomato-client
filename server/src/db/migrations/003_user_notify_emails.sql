-- ============================================================================
-- 003_user_notify_emails — 用 JSONB 数组取代单值 email 字段
-- ============================================================================
--
-- 之前 `users.email` 是单值 TEXT,只能配一个通知邮箱。每个用户实际
-- 经常需要多个邮箱(自己 + 同事/老板各一份),管理员尤其如此。
-- 改成 JSONB 数组之后:
--
--   * 用户可以自己配 0..N 个收件邮箱
--   * 所有的通知 (掉线告警、收益变化、月度收益等) 都会同时发给数组里
--     的每一个地址
--   * 管理员收件人聚合 (email_settings.recipients ∪ 全部 admin 用户的
--     notify_emails) 仍然走 `email_sender::resolve_admin_recipients`
--
-- 数据迁移:把已存在的 `email` (非 NULL 非空) 包装成单元素数组,然后
-- 删旧列。NULL/空字符串 → 空数组。
--
-- CHECK 限制必须是 JSON array 防止其他写入路径误传 object/string;
-- 跟 browser_profiles.tags 同样的 defensive pattern。

ALTER TABLE users
    ADD COLUMN notify_emails JSONB NOT NULL DEFAULT '[]'::jsonb;

UPDATE users
SET notify_emails = jsonb_build_array(email)
WHERE email IS NOT NULL
  AND TRIM(email) <> '';

ALTER TABLE users
    DROP COLUMN email;

ALTER TABLE users
    ADD CONSTRAINT users_notify_emails_is_array
    CHECK (jsonb_typeof(notify_emails) = 'array');
