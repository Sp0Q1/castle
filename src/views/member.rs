use serde::{Deserialize, Serialize};

use crate::models::_entities::{project_members, users};
use crate::views::user::UserSummary;

#[derive(Debug, Deserialize, Serialize)]
pub struct MemberResponse {
    pub id: i32,
    /// The capacity the user was onboarded in: "staff" or "client".
    pub role: String,
    pub user: UserSummary,
    pub created_at: String,
}

impl MemberResponse {
    #[must_use]
    pub fn new(member: &project_members::Model, user: &users::Model) -> Self {
        Self {
            id: member.id,
            role: member.role.clone(),
            user: UserSummary::new(user),
            created_at: member.created_at.to_rfc3339(),
        }
    }
}
