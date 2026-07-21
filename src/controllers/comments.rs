//! Comments let clients discuss findings with the reporting team. A user may
//! comment on (and read the discussion of) a finding only if they can see that
//! finding — which for clients means it must be published.
use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::models::_entities::{comments, findings, project_members, users};
use crate::security::CurrentUser;
use crate::validation::{self, MAX_COMMENT};
use crate::views::comment::CommentResponse;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateCommentParams {
    pub body: String,
}

/// Loads a finding the user is allowed to see, or fails with the appropriate
/// status: 401 if they are not a member, 404 if it is a draft they may not read.
async fn load_viewable_finding(
    ctx: &AppContext,
    user: &users::Model,
    finding_id: i32,
) -> Result<findings::Model> {
    let finding = findings::Entity::find_by_id(finding_id)
        .one(&ctx.db)
        .await?
        .ok_or_else(|| Error::NotFound)?;

    let membership =
        project_members::Model::find_membership(&ctx.db, finding.project_id, user.id).await?;

    if !(user.is_manager() || membership.is_some()) {
        return Err(Error::Unauthorized(
            "you do not have access to this finding".to_string(),
        ));
    }

    let privileged = user.is_manager() || membership.as_ref().is_some_and(|m| m.role == "staff");
    if !privileged && !finding.is_published() {
        return Err(Error::NotFound);
    }

    Ok(finding)
}

#[debug_handler]
pub async fn list(
    CurrentUser(user): CurrentUser,
    Path(finding_id): Path<i32>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let finding = load_viewable_finding(&ctx, &user, finding_id).await?;

    let comment_models = comments::Model::list_for_finding(&ctx.db, finding.id).await?;
    let mut response = Vec::with_capacity(comment_models.len());
    for comment in &comment_models {
        if let Some(commenter) = users::Entity::find_by_id(comment.user_id)
            .one(&ctx.db)
            .await?
        {
            response.push(CommentResponse::new(comment, &commenter));
        }
    }

    format::json(response)
}

#[debug_handler]
pub async fn create(
    CurrentUser(user): CurrentUser,
    Path(finding_id): Path<i32>,
    State(ctx): State<AppContext>,
    Json(params): Json<CreateCommentParams>,
) -> Result<Response> {
    let finding = load_viewable_finding(&ctx, &user, finding_id).await?;

    validation::required_text("comment body", &params.body, MAX_COMMENT)?;

    let comment = comments::ActiveModel {
        finding_id: Set(finding.id),
        user_id: Set(user.id),
        body: Set(params.body),
        ..Default::default()
    }
    .insert(&ctx.db)
    .await?;

    format::json(CommentResponse::new(&comment, &user))
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/findings")
        .add("/{finding_id}/comments", get(list))
        .add("/{finding_id}/comments", post(create))
}
