use serde::Serialize;

use crate::models::_entities::projects;
use crate::models::project_members::Access;

#[derive(Debug, Serialize)]
pub struct ProjectResponse {
    pub id: i64,
    pub pid: String,
    pub name: String,
    pub description: Option<String>,
    pub status: String,
    pub created_by: i64,
    pub created_at: String,
    pub updated_at: String,
    /// What the person asking may do on this project.
    pub access: Access,
}

impl ProjectResponse {
    #[must_use]
    pub fn new(project: &projects::Model, access: Access) -> Self {
        Self {
            id: project.id,
            pid: project.pid.to_string(),
            name: project.name.clone(),
            description: project.description.clone(),
            status: project.status.clone(),
            created_by: project.created_by,
            created_at: project.created_at.to_rfc3339(),
            updated_at: project.updated_at.to_rfc3339(),
            access,
        }
    }
}
