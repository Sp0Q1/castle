//! Everyone with an account in this portal, for management only: who they are,
//! the role the IdP gives them, whether the account is in use, and the projects
//! they are on. Reading only: a role is the IdP's to give, and a membership is
//! changed on its project.
use loco_rs::prelude::*;

use crate::models::_entities::{project_members, projects, users};
use crate::security::CurrentUser;
use crate::views::person::PersonResponse;

#[debug_handler]
pub async fn list(
    CurrentUser(user): CurrentUser,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    if !user.is_manager() {
        return Err(crate::security::forbidden(
            "only management can list everyone",
        ));
    }

    let people = users::Entity::find()
        .order_by_asc(users::Column::Email)
        .all(&ctx.db)
        .await?;
    let memberships = project_members::Entity::find()
        .find_also_related(projects::Entity)
        .order_by_asc(projects::Column::Name)
        .all(&ctx.db)
        .await?;

    let response: Vec<PersonResponse> = people
        .iter()
        .map(|person| PersonResponse::new(person, &memberships))
        .collect();
    format::json(response)
}

pub fn routes() -> Routes {
    Routes::new().prefix("/api/users").add("/", get(list))
}
