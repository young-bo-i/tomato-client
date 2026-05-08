//! 管理员接口：外部 API 请求审计日志。
//!
//! GET  /api/admin/api_log          — 分页列表，支持多维过滤
//! POST /api/admin/api_log/mark     — 批量标记已知/未知
//! DELETE /api/admin/api_log        — 批量删除
//! GET  /api/admin/api_log/export   — 导出当前过滤结果为 CSV

use actix_web::{web, HttpResponse};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::FromRow;

use crate::auth::AdminUser;
use crate::db::DbPool;
use crate::errors::AppResult;

// ── 数据结构 ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, FromRow)]
pub struct ApiLogRow {
    pub id: i64,
    pub service: String,
    pub endpoint: String,
    pub request_summary: Option<JsonValue>,
    pub http_status: Option<i32>,
    pub raw_response: Option<JsonValue>,
    pub parsed_ok: bool,
    pub parse_error: Option<String>,
    pub acknowledged: bool,
    pub acknowledged_at: Option<DateTime<Local>>,
    pub created_at: DateTime<Local>,
}

/// Internal shape for the window-function query: all ApiLogRow fields
/// plus the window-computed total_count so we get count + data in one
/// scan instead of two.
#[derive(Debug, FromRow)]
struct CountedApiLogRow {
    id: i64,
    service: String,
    endpoint: String,
    request_summary: Option<JsonValue>,
    http_status: Option<i32>,
    raw_response: Option<JsonValue>,
    parsed_ok: bool,
    parse_error: Option<String>,
    acknowledged: bool,
    acknowledged_at: Option<DateTime<Local>>,
    created_at: DateTime<Local>,
    total_count: i64,
}

impl From<CountedApiLogRow> for ApiLogRow {
    fn from(r: CountedApiLogRow) -> Self {
        ApiLogRow {
            id: r.id,
            service: r.service,
            endpoint: r.endpoint,
            request_summary: r.request_summary,
            http_status: r.http_status,
            raw_response: r.raw_response,
            parsed_ok: r.parsed_ok,
            parse_error: r.parse_error,
            acknowledged: r.acknowledged,
            acknowledged_at: r.acknowledged_at,
            created_at: r.created_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct PagedApiLog {
    pub rows: Vec<ApiLogRow>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub service: Option<String>,
    pub endpoint: Option<String>,
    /// None = 全部，Some(true) = 只看成功，Some(false) = 只看失败
    pub parsed_ok: Option<bool>,
    /// None = 全部，Some(false) = 未标记（默认视图），Some(true) = 已标记
    pub acknowledged: Option<bool>,
    pub date_from: Option<DateTime<Local>>,
    pub date_to: Option<DateTime<Local>>,
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
}

fn default_page() -> i64 { 1 }
fn default_page_size() -> i64 { 20 }

#[derive(Debug, Deserialize)]
pub struct MarkRequest {
    pub ids: Vec<i64>,
    /// true = 标记为已知，false = 取消标记
    pub acknowledged: bool,
}

#[derive(Debug, Deserialize)]
pub struct DeleteRequest {
    pub ids: Vec<i64>,
}

// ── 查询构建辅助 ──────────────────────────────────────────────────────────

/// 统一的 WHERE 子句条件，List 和 Export 复用同一套过滤逻辑。
/// 返回 (where_sql, binds) 以便与不同 SELECT 组合。
///
/// 注意：sqlx QueryBuilder 不支持动态绑定参数序号，所以我们手动拼接
/// 条件字符串，然后逐步 bind。这里把过滤条件序列化为结构以便两次调用共享。
struct FilterConditions {
    clauses: Vec<String>,
    /// 参数编号从 1 开始，随着条件追加递增
    next_param: i32,
    service: Option<String>,
    endpoint: Option<String>,
    parsed_ok: Option<bool>,
    acknowledged: Option<bool>,
    date_from: Option<DateTime<Local>>,
    date_to: Option<DateTime<Local>>,
}

impl FilterConditions {
    fn new(q: &ListQuery) -> Self {
        let mut s = Self {
            clauses: Vec::new(),
            next_param: 1,
            service: q.service.clone().filter(|s| !s.is_empty()),
            endpoint: q.endpoint.clone().filter(|s| !s.is_empty()),
            parsed_ok: q.parsed_ok,
            acknowledged: q.acknowledged,
            date_from: q.date_from,
            date_to: q.date_to,
        };
        if s.service.is_some() {
            s.clauses.push(format!("service = ${}", s.next_param));
            s.next_param += 1;
        }
        if s.endpoint.is_some() {
            s.clauses.push(format!("endpoint ILIKE ${}", s.next_param));
            s.next_param += 1;
        }
        if s.parsed_ok.is_some() {
            s.clauses.push(format!("parsed_ok = ${}", s.next_param));
            s.next_param += 1;
        }
        if s.acknowledged.is_some() {
            s.clauses.push(format!("acknowledged = ${}", s.next_param));
            s.next_param += 1;
        }
        if s.date_from.is_some() {
            s.clauses.push(format!("created_at >= ${}", s.next_param));
            s.next_param += 1;
        }
        if s.date_to.is_some() {
            s.clauses.push(format!("created_at <= ${}", s.next_param));
            s.next_param += 1;
        }
        s
    }

    fn where_sql(&self) -> String {
        if self.clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", self.clauses.join(" AND "))
        }
    }

    // 链式绑定：按照和 new() 中相同的顺序 bind 所有参数
    fn bind_to<'q, O: sqlx::FromRow<'q, sqlx::postgres::PgRow>>(
        &self,
        mut q: sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments>,
    ) -> sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments>
    {
        if let Some(ref v) = self.service { q = q.bind(v.clone()); }
        if let Some(ref v) = self.endpoint { q = q.bind(format!("%{v}%")); }
        if let Some(v) = self.parsed_ok { q = q.bind(v); }
        if let Some(v) = self.acknowledged { q = q.bind(v); }
        if let Some(v) = self.date_from { q = q.bind(v); }
        if let Some(v) = self.date_to { q = q.bind(v); }
        q
    }

}

// ── 处理器 ───────────────────────────────────────────────────────────────

/// 分页列表。默认按 created_at DESC。
///
/// Uses `COUNT(*) OVER()` window function to get both the page rows and
/// the total count in a single DB scan instead of two separate queries.
pub async fn list(
    pool: web::Data<DbPool>,
    _: AdminUser,
    query: web::Query<ListQuery>,
) -> AppResult<HttpResponse> {
    let page_size = query.page_size.clamp(1, 100);
    let page = query.page.max(1);
    let offset = (page - 1) * page_size;

    let f = FilterConditions::new(&query);
    let where_sql = f.where_sql();

    // LIMIT / OFFSET 参数序号接在过滤参数之后
    let lim_param = f.next_param;
    let off_param = f.next_param + 1;
    let rows_sql = format!(
        r#"SELECT id, service, endpoint, request_summary, http_status,
                  raw_response, parsed_ok, parse_error,
                  acknowledged, acknowledged_at, created_at,
                  COUNT(*) OVER() AS total_count
           FROM external_api_responses
           {where_sql}
           ORDER BY created_at DESC
           LIMIT ${lim_param} OFFSET ${off_param}"#
    );
    let counted: Vec<CountedApiLogRow> = f
        .bind_to(sqlx::query_as(&rows_sql))
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool.get_ref())
        .await?;

    let total = counted.first().map(|r| r.total_count).unwrap_or(0);
    let rows: Vec<ApiLogRow> = counted.into_iter().map(Into::into).collect();

    Ok(HttpResponse::Ok().json(PagedApiLog { rows, total, page, page_size }))
}

/// 批量标记 acknowledged。
pub async fn mark(
    pool: web::Data<DbPool>,
    _: AdminUser,
    body: web::Json<MarkRequest>,
) -> AppResult<HttpResponse> {
    let body = body.into_inner();
    if body.ids.is_empty() {
        return Ok(HttpResponse::Ok().json(serde_json::json!({ "updated": 0 })));
    }
    let ack_at: Option<DateTime<Local>> = if body.acknowledged {
        Some(Local::now())
    } else {
        None
    };
    let result = sqlx::query(
        r#"UPDATE external_api_responses
           SET acknowledged = $1, acknowledged_at = $2
           WHERE id = ANY($3::bigint[])"#,
    )
    .bind(body.acknowledged)
    .bind(ack_at)
    .bind(&body.ids)
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok()
        .json(serde_json::json!({ "updated": result.rows_affected() })))
}

/// 批量删除。只删除 ids 明确传入的行，不支持"删全部"以防误操作。
pub async fn delete(
    pool: web::Data<DbPool>,
    _: AdminUser,
    body: web::Json<DeleteRequest>,
) -> AppResult<HttpResponse> {
    let body = body.into_inner();
    if body.ids.is_empty() {
        return Ok(HttpResponse::Ok().json(serde_json::json!({ "deleted": 0 })));
    }
    let result = sqlx::query(
        "DELETE FROM external_api_responses WHERE id = ANY($1::bigint[])",
    )
    .bind(&body.ids)
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok()
        .json(serde_json::json!({ "deleted": result.rows_affected() })))
}

/// 导出：与 list 使用相同过滤，返回 CSV 字符串包在 JSON 中。
/// 客户端收到后通过 Blob URL 触发下载，避免在 URL 中暴露 token。
/// 最多导出 5000 行防止内存爆炸。
pub async fn export(
    pool: web::Data<DbPool>,
    _: AdminUser,
    query: web::Query<ListQuery>,
) -> AppResult<HttpResponse> {
    let f = FilterConditions::new(&query);
    let where_sql = f.where_sql();
    let lim_param = f.next_param;

    let rows_sql = format!(
        r#"SELECT id, service, endpoint, request_summary, http_status,
                  raw_response, parsed_ok, parse_error,
                  acknowledged, acknowledged_at, created_at
           FROM external_api_responses
           {where_sql}
           ORDER BY created_at DESC
           LIMIT ${lim_param}"#
    );
    let rows: Vec<ApiLogRow> = f
        .bind_to(sqlx::query_as(&rows_sql))
        .bind(5000_i64)
        .fetch_all(pool.get_ref())
        .await?;

    let csv = build_csv(&rows);
    Ok(HttpResponse::Ok()
        .json(serde_json::json!({ "csv": csv, "count": rows.len() })))
}

fn build_csv(rows: &[ApiLogRow]) -> String {
    let mut out = String::from(
        "id,created_at,service,endpoint,http_status,parsed_ok,acknowledged,parse_error,request_summary,raw_response\n",
    );
    for r in rows {
        out.push_str(&csv_field(&r.id.to_string()));
        out.push(',');
        out.push_str(&csv_field(&r.created_at.format("%Y-%m-%dT%H:%M:%S").to_string()));
        out.push(',');
        out.push_str(&csv_field(&r.service));
        out.push(',');
        out.push_str(&csv_field(&r.endpoint));
        out.push(',');
        out.push_str(&csv_field(&r.http_status.map(|s| s.to_string()).unwrap_or_default()));
        out.push(',');
        out.push_str(&csv_field(if r.parsed_ok { "true" } else { "false" }));
        out.push(',');
        out.push_str(&csv_field(if r.acknowledged { "true" } else { "false" }));
        out.push(',');
        out.push_str(&csv_field(r.parse_error.as_deref().unwrap_or("")));
        out.push(',');
        out.push_str(&csv_field(
            &r.request_summary
                .as_ref()
                .map(|v| v.to_string())
                .unwrap_or_default(),
        ));
        out.push(',');
        out.push_str(&csv_field(
            &r.raw_response
                .as_ref()
                .map(|v| v.to_string())
                .unwrap_or_default(),
        ));
        out.push('\n');
    }
    out
}

/// RFC 4180 CSV 字段转义：包含逗号/引号/换行时加双引号，引号本身翻倍。
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}
