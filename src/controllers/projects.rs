//! Projects are opened by management and are the container into which staff and
//! clients are onboarded. Access rules enforced here:
//!   * create a project ...... management only
//!   * onboard a member ...... management only
//!   * view a project ........ management, or a user onboarded to it
use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::models::_entities::{project_members, projects, users};
use crate::security::CurrentUser;
use crate::validation::{self, MAX_DESCRIPTION, MAX_EMAIL, MAX_TITLE};
use crate::views::member::MemberResponse;
use crate::views::project::ProjectResponse;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateProjectParams {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OnboardParams {
    /// Email of the existing user to add to the project.
    pub user_email: String,
    /// Capacity to onboard them in: "staff" or "client".
    pub role: String,
}

/// Loads a project by id or returns 404.
async fn load_project(ctx: &AppContext, id: i32) -> Result<projects::Model> {
    projects::Entity::find_by_id(id)
        .one(&ctx.db)
        .await?
        .ok_or_else(|| Error::NotFound)
}

/// Whether `user` may view `project_id`: management sees every project, everyone
/// else only the projects they have been onboarded to.
async fn can_view(ctx: &AppContext, user: &users::Model, project_id: i32) -> Result<bool> {
    if user.is_manager() {
        return Ok(true);
    }
    Ok(project_members::Model::is_member(&ctx.db, project_id, user.id).await?)
}

#[debug_handler]
pub async fn create(
    CurrentUser(user): CurrentUser,
    State(ctx): State<AppContext>,
    Json(params): Json<CreateProjectParams>,
) -> Result<Response> {
    if !user.is_manager() {
        return unauthorized("only management can create projects");
    }

    validation::required_text("name", &params.name, MAX_TITLE)?;
    if let Some(description) = &params.description {
        validation::text("description", description, MAX_DESCRIPTION)?;
    }

    let project = projects::ActiveModel {
        name: Set(params.name),
        description: Set(params.description),
        created_by: Set(user.id),
        ..Default::default()
    }
    .insert(&ctx.db)
    .await?;

    format::json(ProjectResponse::new(&project))
}

#[debug_handler]
pub async fn list(
    CurrentUser(user): CurrentUser,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let projects = if user.is_manager() {
        projects::Entity::find().all(&ctx.db).await?
    } else {
        let memberships = project_members::Entity::find()
            .filter(project_members::Column::UserId.eq(user.id))
            .all(&ctx.db)
            .await?;
        let ids: Vec<i32> = memberships.iter().map(|m| m.project_id).collect();
        if ids.is_empty() {
            Vec::new()
        } else {
            projects::Entity::find()
                .filter(projects::Column::Id.is_in(ids))
                .all(&ctx.db)
                .await?
        }
    };

    let response: Vec<ProjectResponse> = projects.iter().map(ProjectResponse::new).collect();
    format::json(response)
}

#[debug_handler]
pub async fn show(
    CurrentUser(user): CurrentUser,
    Path(project_id): Path<i32>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let project = load_project(&ctx, project_id).await?;
    if !can_view(&ctx, &user, project.id).await? {
        return unauthorized("you do not have access to this project");
    }
    format::json(ProjectResponse::new(&project))
}

#[debug_handler]
pub async fn onboard(
    CurrentUser(user): CurrentUser,
    Path(project_id): Path<i32>,
    State(ctx): State<AppContext>,
    Json(params): Json<OnboardParams>,
) -> Result<Response> {
    if !user.is_manager() {
        return unauthorized("only management can onboard members");
    }
    let project = load_project(&ctx, project_id).await?;

    validation::required_text("user_email", &params.user_email, MAX_EMAIL)?;

    let role = match params.role.as_str() {
        "staff" | "client" => params.role.clone(),
        _ => return bad_request("role must be either 'staff' or 'client'"),
    };

    // Create an "invited" placeholder if the user has not signed in yet; it is
    // reconciled to their real identity on first SSO login (matched by email).
    let target =
        users::Model::find_or_invite(&ctx.db, &params.user_email.trim().to_lowercase()).await?;

    if project_members::Model::is_member(&ctx.db, project.id, target.id).await? {
        return bad_request("that user is already a member of this project");
    }

    let member = project_members::ActiveModel {
        project_id: Set(project.id),
        user_id: Set(target.id),
        role: Set(role),
        ..Default::default()
    }
    .insert(&ctx.db)
    .await?;

    format::json(MemberResponse::new(&member, &target))
}

#[debug_handler]
pub async fn list_members(
    CurrentUser(user): CurrentUser,
    Path(project_id): Path<i32>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let project = load_project(&ctx, project_id).await?;
    if !can_view(&ctx, &user, project.id).await? {
        return unauthorized("you do not have access to this project");
    }

    let members = project_members::Model::list_for_project(&ctx.db, project.id).await?;
    let mut response = Vec::with_capacity(members.len());
    for member in &members {
        if let Some(member_user) = users::Entity::find_by_id(member.user_id)
            .one(&ctx.db)
            .await?
        {
            response.push(MemberResponse::new(member, &member_user));
        }
    }

    format::json(response)
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/projects")
        .add("/", post(create))
        .add("/", get(list))
        .add("/{project_id}", get(show))
        .add("/{project_id}/members", post(onboard))
        .add("/{project_id}/members", get(list_members))
}
