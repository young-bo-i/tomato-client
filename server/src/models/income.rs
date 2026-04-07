use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct KolIncome {
    pub id: i64,
    pub account_id: i32,
    pub kol_id: i32,
    pub total_income: i64,          // in cents
    pub regular_income: i64,
    pub bonus_income: i64,
    pub current_month_income: i64,
    pub current_week_income: i64,
    pub income_json: Option<String>,
    pub monthly_income_list_json: Option<String>,
    pub weekly_income_list_json: Option<String>,
    pub last_update_time: NaiveDateTime,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct IncomeNotice {
    pub id: i32,
    pub account_id: i32,
    pub email: String,
    pub has_child: bool,
    pub is_deleted: bool,
    pub created_at: NaiveDateTime,
}
