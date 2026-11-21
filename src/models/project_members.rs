use loco_rs::prelude::*;
use sea_orm::entity::prelude::{DeriveActiveEnum, EnumIter, StringLen};
use serde::{Deserialize, Serialize};

pub use super::_entities::project_members::{ActiveModel, Column, Entity, Model};
use super::_entities::{findings, users};
use super::users::UserRole;

/// The capacity a manager put a user in *on one project* — distinct from their
/// platform-wide [`crate::models::users::UserRole`], which an IdP group decides.
/// What someone may do follows from both: see [`Access`]. Management is not a
/// capacity: a manager's access comes from the role, on every project, so there
/// is no value for it here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
#[serde(rename_all = "lowercase")]
pub enum MemberRole {
    /// May author, edit and publish findings.
    #[sea_orm(string_value = "staff")]
    Staff,
    /// May read published findings and comment.
    #[sea_orm(string_value = "client")]
    Client,
}

impl MemberRole {
    /// Parses a caller-supplied capacity.
    ///
    /// # Errors
    /// When `value` names anything other than the capacities a membership can hold.
    pub fn parse(value: &str) -> std::result::Result<Self, String> {
        match value {
            "staff" => Ok(Self::Staff),
            "client" => Ok(Self::Client),
            _ => Err("role must be one of: staff, client".to_string()),
        }
    }
}

/// What someone may do on one project. It follows from two facts, the role the
/// IdP gives them and the capacity management put them in, and it is worked out
/// here and nowhere else: every handler asks for it, and the page is sent the
/// answer rather than the facts to work it out again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    /// Reads and writes everything on the project, drafts included.
    Staff,
    /// Reads published findings and joins their discussion.
    Client,
}

impl Access {
    /// What `user` may do on the project `membership` is for. Staff takes the
    /// IdP's word that they are staff and management's that they work on this
    /// project; neither alone will do.
    #[must_use]
    pub fn as_member(user: &users::Model, membership: &Model) -> Self {
        match (user.role(), membership.role) {
            (UserRole::Manager, _) | (UserRole::Staff, MemberRole::Staff) => Self::Staff,
            (UserRole::Staff | UserRole::Client, _) => Self::Client,
        }
    }

    /// What `user` may do on a project, given their membership of it if they
    /// have one. `None`: the project does not exist for them. Management is on
    /// no project and opens every one.
    #[must_use]
    pub fn of(user: &users::Model, membership: Option<&Model>) -> Option<Self> {
        match membership {
            Some(membership) => Some(Self::as_member(user, membership)),
            None => user.is_manager().then_some(Self::Staff),
        }
    }

    /// What `user` may do on `project_id`.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn to_project(
        db: &DatabaseConnection,
        user: &users::Model,
        project_id: i64,
    ) -> ModelResult<Option<Self>> {
        let membership = Model::find_membership(db, project_id, user.id).await?;
        Ok(Self::of(user, membership.as_ref()))
    }

    /// Whether `finding` exists for someone with this access: a draft does not
    /// for a client.
    #[must_use]
    pub fn sees(self, finding: &findings::Model) -> bool {
        self == Self::Staff || finding.is_published()
    }
}

#[async_trait::async_trait]
impl ActiveModelBehavior for super::_entities::project_members::ActiveModel {
    async fn before_save<C>(self, _db: &C, insert: bool) -> std::result::Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        if !insert && self.updated_at.is_unchanged() {
            let mut this = self;
            this.updated_at = ActiveValue::Set(chrono::Utc::now().into());
            Ok(this)
        } else {
            Ok(self)
        }
    }
}

// read-oriented logic
impl Model {
    /// Returns the membership row for `user_id` in `project_id`, if the user is
    /// onboarded to that project.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn find_membership(
        db: &DatabaseConnection,
        project_id: i64,
        user_id: i64,
    ) -> ModelResult<Option<Self>> {
        let member = Entity::find()
            .filter(Column::ProjectId.eq(project_id))
            .filter(Column::UserId.eq(user_id))
            .one(db)
            .await?;
        Ok(member)
    }

    /// Whether `user_id` is a member of `project_id`.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn is_member(
        db: &DatabaseConnection,
        project_id: i64,
        user_id: i64,
    ) -> ModelResult<bool> {
        Ok(Self::find_membership(db, project_id, user_id)
            .await?
            .is_some())
    }

    /// Lists every membership row for a project, each with the user it names.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn list_for_project_with_users(
        db: &DatabaseConnection,
        project_id: i64,
    ) -> ModelResult<Vec<(Self, Option<users::Model>)>> {
        let members = Entity::find()
            .filter(Column::ProjectId.eq(project_id))
            .find_also_related(users::Entity)
            .all(db)
            .await?;
        Ok(members)
    }
}
