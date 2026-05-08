//! 邮件模板工具:统一所有通知邮件的视觉风格,在桌面 + 移动端都好看。
//!
//! 核心约束(决定为什么这么写):
//!   * 邮件客户端会**丢弃 `<head><style>`** (Gmail 移动版尤其严重),所以
//!     全部用**内联样式**。
//!   * 不用 flexbox / float / position(老客户端不支持),全部用嵌套 table。
//!   * 容器 `max-width: 600px`,小屏自动 100% 宽,用 viewport meta 阻止
//!     iOS Mail 的自动缩放(`x-apple-disable-message-reformatting`)。
//!   * 字号 ≥ 13px(Gmail 移动版会强制 ≥ 13px),正文 14-15px。
//!   * 颜色用浅灰背景 + 白色卡片,中性灰文字 — 尊重深色模式不会显得突兀。
//!   * 中文优先用 PingFang (iOS) / Microsoft YaHei (Win) / 系统默认 sans。
//!
//! API 设计成"小积木":
//!   * `email_shell(title, subtitle, content_html, footer)` — 外层骨架
//!   * `card(primary, secondary, fields)` — 一条记录的卡片(替代宽表格)
//!   * `notice_card(primary, secondary, embedded_html)` — 嵌入第三方 HTML 的卡片
//!   * `html_escape(s)` — 转义用户/profile 名等控制字段
//!
//! 不复用 component crate 的原因:渲染逻辑非常薄,直接写字符串比拉一个
//! handlebars / askama 依赖更轻。

use std::fmt::Write as _;

/// 一个 key-value 字段。`highlight = true` 时高亮显示(粗体 + 强调色),
/// 用于"本次新增"、"+¥xxx"这种最重要的数字。`value` 已经做过 HTML 转义。
pub struct Field<'a> {
    pub label: &'a str,
    pub value: &'a str,
    pub highlight: bool,
}

/// 外层骨架。所有 HTML 邮件都套在这里 — 让宽度 / 字号 / 配色统一。
pub fn email_shell(
    title: &str,
    subtitle: Option<&str>,
    content_html: &str,
    footer: Option<&str>,
) -> String {
    let subtitle_html = subtitle
        .filter(|s| !s.is_empty())
        .map(|s| {
            format!(
                r#"<p style="margin:6px 0 0;font-size:13px;line-height:1.55;color:#5f6368;">{}</p>"#,
                html_escape(s)
            )
        })
        .unwrap_or_default();

    let footer_html = footer
        .filter(|s| !s.is_empty())
        .map(|s| html_escape(s))
        .unwrap_or_default();

    format!(
        r#"<!doctype html>
<html lang="zh">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="x-apple-disable-message-reformatting">
<meta name="format-detection" content="telephone=no,date=no,address=no,email=no">
<title>{title}</title>
</head>
<body style="margin:0;padding:0;background:#f5f6f8;font-family:-apple-system,BlinkMacSystemFont,'Helvetica Neue','PingFang SC','Hiragino Sans GB','Microsoft YaHei',sans-serif;color:#1f2933;line-height:1.55;-webkit-text-size-adjust:100%;">
<center style="width:100%;background:#f5f6f8;padding:16px 0;">
<table role="presentation" cellspacing="0" cellpadding="0" border="0" align="center" width="100%" style="max-width:600px;margin:0 auto;background:#ffffff;border-radius:12px;box-shadow:0 1px 4px rgba(0,0,0,0.06);">
  <tr><td style="padding:20px 20px 0;">
    <h1 style="margin:0;font-size:18px;line-height:1.4;color:#1a73e8;font-weight:600;">{title_escaped}</h1>
    {subtitle_html}
  </td></tr>
  <tr><td style="padding:14px 20px 6px;">
{content_html}
  </td></tr>
  <tr><td style="padding:14px 20px 20px;border-top:1px solid #eef0f3;color:#9aa0a6;font-size:12px;line-height:1.5;text-align:center;">
{footer_html}
  </td></tr>
</table>
</center>
</body>
</html>"#,
        title = html_escape(title),
        title_escaped = html_escape(title),
        subtitle_html = subtitle_html,
        content_html = content_html,
        footer_html = footer_html,
    )
}

/// 标准记录卡片:顶部 primary/secondary 标题区,下面 key-value 行。
/// 高亮字段(highlight=true)用绿色 + 粗体,适合显示"+¥xxx"的金额。
pub fn card(primary: &str, secondary: Option<&str>, fields: &[Field<'_>]) -> String {
    let mut html = String::new();

    let secondary_html = secondary
        .filter(|s| !s.is_empty())
        .map(|s| {
            format!(
                r#"<div style="font-size:12px;line-height:1.45;color:#5f6368;margin-top:3px;">{}</div>"#,
                html_escape(s)
            )
        })
        .unwrap_or_default();

    let _ = write!(
        html,
        r#"<table role="presentation" cellspacing="0" cellpadding="0" border="0" width="100%" style="margin:10px 0;border:1px solid #e8eaed;border-radius:10px;background:#ffffff;border-collapse:separate;">
<tr><td style="padding:11px 14px;background:#f8f9fa;border-bottom:1px solid #e8eaed;border-radius:10px 10px 0 0;">
<div style="font-weight:600;font-size:14px;line-height:1.4;color:#202124;word-break:break-all;">{primary}</div>
{secondary_html}
</td></tr>
<tr><td style="padding:6px 14px 10px;">
<table role="presentation" cellspacing="0" cellpadding="0" border="0" width="100%" style="font-size:13px;line-height:1.6;">"#,
        primary = html_escape(primary),
        secondary_html = secondary_html,
    );

    for f in fields {
        let value_style = if f.highlight {
            "text-align:right;padding:4px 0;font-weight:700;color:#0a8754;font-size:14px;font-variant-numeric:tabular-nums;"
        } else {
            "text-align:right;padding:4px 0;font-weight:600;color:#202124;font-variant-numeric:tabular-nums;"
        };
        let _ = write!(
            html,
            r#"<tr>
<td style="color:#5f6368;padding:4px 0;width:50%;">{label}</td>
<td style="{value_style}">{value}</td>
</tr>"#,
            label = html_escape(f.label),
            value_style = value_style,
            value = f.value, // already escaped by caller
        );
    }

    let _ = write!(html, "</table></td></tr></table>");
    html
}

/// 嵌入第三方 HTML 的卡片(七猫的月度通知就是上游推下来的整段 HTML)。
/// 顶部我们加一个标题区给上下文(用户/账号/日期),下面是上游内容的
/// 沙盒容器 — 用 word-break + max-width 防止上游表格撑破我们的 600px。
pub fn notice_card(
    primary: &str,
    secondary: Option<&str>,
    embedded_html: &str,
) -> String {
    let secondary_html = secondary
        .filter(|s| !s.is_empty())
        .map(|s| {
            format!(
                r#"<div style="font-size:12px;line-height:1.45;color:#5f6368;margin-top:3px;">{}</div>"#,
                html_escape(s)
            )
        })
        .unwrap_or_default();

    format!(
        r#"<table role="presentation" cellspacing="0" cellpadding="0" border="0" width="100%" style="margin:10px 0;border:1px solid #e8eaed;border-radius:10px;background:#ffffff;border-collapse:separate;">
<tr><td style="padding:11px 14px;background:#f8f9fa;border-bottom:1px solid #e8eaed;border-radius:10px 10px 0 0;">
<div style="font-weight:600;font-size:14px;line-height:1.4;color:#202124;">{primary}</div>
{secondary_html}
</td></tr>
<tr><td style="padding:14px;font-size:13px;line-height:1.6;color:#202124;word-break:break-word;overflow-wrap:break-word;">
{embedded_html}
</td></tr>
</table>"#,
        primary = html_escape(primary),
        secondary_html = secondary_html,
        embedded_html = embedded_html,
    )
}

/// 简单的 HTML 实体转义。所有用户控制的字符串经过这里再放进模板。
pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
