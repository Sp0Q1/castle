use loco_rs::prelude::*;
use uuid::Uuid;

pub use super::_entities::projects::{ActiveModel, Column, Entity, Model};

pub type Projects = Entity;

#[async_trait::async_trait]
impl ActiveModelBehavior for super::_entities::projects::ActiveModel {
    async fn before_save<C>(self, _db: &C, insert: bool) -> std::result::Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        let mut this = self;
        if insert {
            this.pid = ActiveValue::Set(Uuid::new_v4());
        } else if this.updated_at.is_unchanged() {
            this.updated_at = ActiveValue::Set(chrono::Utc::now().into());
        }
        Ok(this)
    }
}

// read-oriented logic
impl Model {
    /// Finds a project by its public id (`pid`).
    ///
    /// # Errors
    /// When the pid is malformed, the project is missing, or the query fails.
    pub async fn find_by_pid(db: &DatabaseConnection, pid: &str) -> ModelResult<Self> {
        let uuid = Uuid::parse_str(pid).map_err(|e| ModelError::Any(e.into()))?;
        let project = Entity::find().filter(Column::Pid.eq(uuid)).one(db).await?;
        project.ok_or_else(|| ModelError::EntityNotFound)
    }

    /// Loads a project by its numeric id.
    ///
    /// # Errors
    /// When the project is missing or the query fails.
    pub async fn find_by_id(db: &DatabaseConnection, id: i32) -> ModelResult<Self> {
        let project = Entity::find_by_id(id).one(db).await?;
        project.ok_or_else(|| ModelError::EntityNotFound)
    }
}
