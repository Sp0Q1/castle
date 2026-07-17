use loco_rs::prelude::*;
use uuid::Uuid;

pub use super::_entities::findings::{ActiveModel, Column, Entity, Model};

pub type Findings = Entity;

/// A finding is only exposed to clients once it has been published.
pub const STATUS_DRAFT: &str = "draft";
pub const STATUS_PUBLISHED: &str = "published";

#[async_trait::async_trait]
impl ActiveModelBehavior for super::_entities::findings::ActiveModel {
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
    /// Finds a finding by its public id (`pid`).
    ///
    /// # Errors
    /// When the pid is malformed, the finding is missing, or the query fails.
    pub async fn find_by_pid(db: &DatabaseConnection, pid: &str) -> ModelResult<Self> {
        let uuid = Uuid::parse_str(pid).map_err(|e| ModelError::Any(e.into()))?;
        let finding = Entity::find().filter(Column::Pid.eq(uuid)).one(db).await?;
        finding.ok_or_else(|| ModelError::EntityNotFound)
    }

    /// Loads a finding by its numeric id.
    ///
    /// # Errors
    /// When the finding is missing or the query fails.
    pub async fn find_by_id(db: &DatabaseConnection, id: i32) -> ModelResult<Self> {
        let finding = Entity::find_by_id(id).one(db).await?;
        finding.ok_or_else(|| ModelError::EntityNotFound)
    }

    /// Lists the findings that belong to a project.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn list_for_project(
        db: &DatabaseConnection,
        project_id: i32,
    ) -> ModelResult<Vec<Self>> {
        let findings = Entity::find()
            .filter(Column::ProjectId.eq(project_id))
            .all(db)
            .await?;
        Ok(findings)
    }

    /// Whether this finding is visible to clients.
    #[must_use]
    pub fn is_published(&self) -> bool {
        self.status == STATUS_PUBLISHED
    }
}
