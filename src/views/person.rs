use serde::Serialize;

use crate::models::_entities::{project_members, projects, users};
use crate::models::project_members::MemberRole;
use crate::models::users::UserStatus;

/// One project someone was put on, and as what.
#[derive(Debug, Serialize)]
pub struct PersonProject {
    pub id: i64,
    pub name: String,
    pub role: MemberRole,
}

/// Someone with an account, as management sees them: who they are, the role
/// the IdP gives them, and the projects they were put on. A manager opens every
/// project by role and needs no entry here. Like every view of a user, it
/// carries no credential.
#[derive(Debug, Serialize)]
pub struct PersonResponse {
    pub id: i64,
    pub pid: String,
    pub name: String,
    pub email: String,
    pub role: String,
    pub status: UserStatus,
    pub created_at: String,
    pub projects: Vec<PersonProject>,
}

impl PersonResponse {
    #[must_use]
    pub fn new(
        user: &users::Model,
        memberships: &[(project_members::Model, Option<projects::Model>)],
    ) -> Self {
        Self {
            id: user.id,
            pid: user.pid.to_string(),
            name: user.name.clone(),
            email: user.email.clone(),
            role: user.role.clone(),
            status: user.status,
            created_at: user.created_at.to_rfc3339(),
            projects: memberships
                .iter()
                // An older row naming who opened a project is not a membership.
                .filter(|(member, _)| {
                    member.user_id == user.id && member.role != MemberRole::Manager
                })
                .filter_map(|(member, project)| {
                    project.as_ref().map(|project| PersonProject {
                        id: project.id,
                        name: project.name.clone(),
                        role: member.role,
                    })
                })
                .collect(),
        }
    }
}
