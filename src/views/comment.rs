use serde::{Deserialize, Serialize};

use crate::models::_entities::{comments, users};
use crate::views::user::UserSummary;

#[derive(Debug, Deserialize, Serialize)]
pub struct CommentResponse {
    pub id: i32,
    pub body: String,
    pub author: UserSummary,
    pub created_at: String,
}

impl CommentResponse {
    #[must_use]
    pub fn new(comment: &comments::Model, author: &users::Model) -> Self {
        Self {
            id: comment.id,
            body: comment.body.clone(),
            author: UserSummary::new(author),
            created_at: comment.created_at.to_rfc3339(),
        }
    }
}
