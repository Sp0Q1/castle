use loco_rs::prelude::*;

pub use super::_entities::project_members::{ActiveModel, Column, Entity, Model};

pub type ProjectMembers = Entity;

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
        project_id: i32,
        user_id: i32,
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
        project_id: i32,
        user_id: i32,
    ) -> ModelResult<bool> {
        Ok(Self::find_membership(db, project_id, user_id)
            .await?
            .is_some())
    }

    /// Lists every membership row for a project.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn list_for_project(
        db: &DatabaseConnection,
        project_id: i32,
    ) -> ModelResult<Vec<Self>> {
        let members = Entity::find()
            .filter(Column::ProjectId.eq(project_id))
            .all(db)
            .await?;
        Ok(members)
    }
}
