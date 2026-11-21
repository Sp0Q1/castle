pub mod auth;
pub mod brand;
pub mod comments;
pub mod findings;
pub mod people;
pub mod projects;
pub mod uploads;

use loco_rs::prelude::*;

use crate::models::_entities::users;
use crate::models::project_members::Access;

/// The policy of a file that is served as it is, an upload or the logo: it
/// loads nothing, and opened as a document it runs nothing. A header a handler
/// sets is kept by the app-wide `secure_headers` layer, which fills in only
/// what is missing.
pub(crate) const FILE_POLICY: &str = "default-src 'none'; sandbox";

/// What `user` may do on `project_id`. A project they cannot open does not
/// exist for them: **404**, never 403, so its existence is not disclosed.
pub(crate) async fn access(
    ctx: &AppContext,
    user: &users::Model,
    project_id: i64,
) -> Result<Access> {
    Access::to_project(&ctx.db, user, project_id)
        .await?
        .ok_or_else(|| Error::NotFound)
}
