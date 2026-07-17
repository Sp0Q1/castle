use serde::{Deserialize, Serialize};

use crate::models::_entities::users;

/// A safe projection of a user for embedding in API responses.
///
/// Deliberately omits sensitive columns (`password`, `api_key`, verification
/// and reset tokens) so user records can be referenced from findings, comments
/// and memberships without leaking credentials.
#[derive(Debug, Deserialize, Serialize)]
pub struct UserSummary {
    pub id: i32,
    pub pid: String,
    pub name: String,
    pub email: String,
    pub role: String,
    pub status: String,
}

impl UserSummary {
    #[must_use]
    pub fn new(user: &users::Model) -> Self {
        Self {
            id: user.id,
            pid: user.pid.to_string(),
            name: user.name.clone(),
            email: user.email.clone(),
            role: user.role.clone(),
            status: user.status.clone(),
        }
    }
}
