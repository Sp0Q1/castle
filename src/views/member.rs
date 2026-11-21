use serde::Serialize;

use crate::models::_entities::{project_members, users};
use crate::models::project_members::{Access, MemberRole};
use crate::views::user::UserSummary;

#[derive(Debug, Serialize)]
pub struct MemberResponse {
    pub id: i64,
    pub role: MemberRole,
    /// What this member may do on the project, which is the capacity above
    /// only as far as the IdP's role for them allows.
    pub access: Access,
    pub user: UserSummary,
    pub created_at: String,
}

impl MemberResponse {
    #[must_use]
    pub fn new(member: &project_members::Model, user: &users::Model) -> Self {
        Self {
            id: member.id,
            role: member.role,
            access: Access::as_member(user, member),
            user: UserSummary::new(user),
            created_at: member.created_at.to_rfc3339(),
        }
    }
}
