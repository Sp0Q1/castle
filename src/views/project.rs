use serde::{Deserialize, Serialize};

use crate::models::_entities::projects;

#[derive(Debug, Deserialize, Serialize)]
pub struct ProjectResponse {
    pub id: i32,
    pub pid: String,
    pub name: String,
    pub description: Option<String>,
    pub status: String,
    pub created_by: i32,
    pub created_at: String,
    pub updated_at: String,
}

impl ProjectResponse {
    #[must_use]
    pub fn new(project: &projects::Model) -> Self {
        Self {
            id: project.id,
            pid: project.pid.to_string(),
            name: project.name.clone(),
            description: project.description.clone(),
            status: project.status.clone(),
            created_by: project.created_by,
            created_at: project.created_at.to_rfc3339(),
            updated_at: project.updated_at.to_rfc3339(),
        }
    }
}
