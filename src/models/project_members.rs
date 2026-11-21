use loco_rs::prelude::*;
use sea_orm::entity::prelude::{DeriveActiveEnum, EnumIter, StringLen};
use serde::{Deserialize, Serialize};

pub use super::_entities::project_members::{ActiveModel, Column, Entity, Model};
use super::_entities::users;

/// The capacity a manager put a user in *on one project* — distinct from their
/// platform-wide [`crate::models::users::UserRole`], which an IdP group decides.
/// Finding authorization asks both: see
/// [`crate::models::users::Model::is_project_staff`]. Management is not a
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
