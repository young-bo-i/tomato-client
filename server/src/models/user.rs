use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    User,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::User => "user",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "admin" => Some(Role::Admin),
            "user" => Some(Role::User),
            _ => None,
        }
    }
}

/// Full user record — NEVER return directly via API (contains password_hash).
#[derive(Debug, FromRow)]
pub struct User {
    pub id: i32,
    pub username: String,
    pub password_hash: String,
    pub role: String,
    pub is_active: bool,
    /// Notification destination for offline alerts. Nullable — admins
    /// without an email are silently skipped by the dispatcher.
    pub email: Option<String>,
    /// 2-level user hierarchy (introduced in migration 003):
    ///   * NULL  → tier-1 (top-level user, or admin)
    ///   * NOT NULL → tier-2; references the tier-1 above this user
    /// API validators forbid 3-level chains (parent must itself have
    /// parent_user_id IS NULL). Admin rows always have NULL.
    pub parent_user_id: Option<i32>,
    /// Rate (0..=100, validated to {0,10,20,50,100} at the API) at
    /// which THIS user's tier-2 subordinates' words flow up to THIS
    /// user. Only meaningful when this user is a tier-1 with at
    /// least one tier-2 subordinate; admin / tier-2 rows leave it 0.
    pub tier2_contribution_pct: i32,
    pub created_at: DateTime<Local>,
    pub updated_at: DateTime<Local>,
}

/// Safe projection returned to clients. Includes tier hierarchy and
/// the dynamically-computed `has_subordinates` flag (the latter is
/// filled in by handlers that JOIN against the parent_user_id index).
#[derive(Debug, Serialize)]
pub struct UserView {
    pub id: i32,
    pub username: String,
    pub role: String,
    pub is_active: bool,
    pub email: Option<String>,
    pub parent_user_id: Option<i32>,
    /// Username of the parent (when this row is tier-2). Filled in by
    /// the admin list query via LEFT JOIN; None for tier-1.
    pub parent_username: Option<String>,
    pub tier2_contribution_pct: i32,
    /// True when at least one other row has `parent_user_id = self.id`.
    /// Determines whether the team-management UI should render for
    /// this user. Filled in by handlers that need it; bare
    /// `From<User> for UserView` defaults it to false.
    pub has_subordinates: bool,
    pub created_at: DateTime<Local>,
    pub updated_at: DateTime<Local>,
}

impl From<User> for UserView {
    fn from(u: User) -> Self {
        Self {
            id: u.id,
            username: u.username,
            role: u.role,
            is_active: u.is_active,
            email: u.email,
            parent_user_id: u.parent_user_id,
            parent_username: None,
            tier2_contribution_pct: u.tier2_contribution_pct,
            has_subordinates: false,
            created_at: u.created_at,
            updated_at: u.updated_at,
        }
    }
}
