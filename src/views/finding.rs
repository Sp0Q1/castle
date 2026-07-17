use serde::{Deserialize, Serialize};

use crate::models::_entities::findings;
use crate::views::comment::CommentResponse;
use crate::views::user::UserSummary;

#[derive(Debug, Deserialize, Serialize)]
pub struct FindingResponse {
    pub id: i32,
    pub pid: String,
    pub project_id: i32,
    pub author_id: i32,
    pub title: String,
    pub finding_type: String,
    pub description: String,
    pub technical_description: String,
    pub impact: String,
    pub recommendation: String,
    pub severity: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

impl FindingResponse {
    #[must_use]
    pub fn new(finding: &findings::Model) -> Self {
        Self {
            id: finding.id,
            pid: finding.pid.to_string(),
            project_id: finding.project_id,
            author_id: finding.author_id,
            title: finding.title.clone(),
            finding_type: finding.finding_type.clone(),
            description: finding.description.clone(),
            technical_description: finding.technical_description.clone(),
            impact: finding.impact.clone(),
            recommendation: finding.recommendation.clone(),
            severity: finding.severity.clone(),
            status: finding.status.clone(),
            created_at: finding.created_at.to_rfc3339(),
            updated_at: finding.updated_at.to_rfc3339(),
        }
    }
}

/// A finding together with its author and the discussion thread — returned by
/// the finding "show" endpoint.
#[derive(Debug, Serialize)]
pub struct FindingDetailResponse {
    #[serde(flatten)]
    pub finding: FindingResponse,
    pub author: UserSummary,
    pub comments: Vec<CommentResponse>,
}

impl FindingDetailResponse {
    #[must_use]
    pub fn new(
        finding: &findings::Model,
        author: &crate::models::_entities::users::Model,
        comments: Vec<CommentResponse>,
    ) -> Self {
        Self {
            finding: FindingResponse::new(finding),
            author: UserSummary::new(author),
            comments,
        }
    }
}
