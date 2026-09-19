use loco_rs::prelude::*;
use sea_orm::QueryOrder;

pub use super::_entities::comments::{ActiveModel, Column, Entity, Model};
use super::_entities::users;

#[async_trait::async_trait]
impl ActiveModelBehavior for super::_entities::comments::ActiveModel {
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
    /// Lists the comments on a finding, oldest first, each with its author.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn list_for_finding_with_authors(
        db: &DatabaseConnection,
        finding_id: i64,
    ) -> ModelResult<Vec<(Self, Option<users::Model>)>> {
        let comments = Entity::find()
            .filter(Column::FindingId.eq(finding_id))
            .order_by_asc(Column::CreatedAt)
            .find_also_related(users::Entity)
            .all(db)
            .await?;
        Ok(comments)
    }
}
