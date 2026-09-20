use loco_rs::prelude::*;
use sea_orm::entity::prelude::{ActiveEnum, DeriveActiveEnum, EnumIter, StringLen};
use sea_orm::Iterable;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use super::_entities::findings::{ActiveModel, Column, Entity, Model};

/// How severe a finding is. Stored as the lowercase name, which is also the
/// wire format the SPA sends and receives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    #[sea_orm(string_value = "low")]
    Low,
    #[sea_orm(string_value = "medium")]
    Medium,
    #[sea_orm(string_value = "elevated")]
    Elevated,
    #[sea_orm(string_value = "high")]
    High,
    #[sea_orm(string_value = "extreme")]
    Extreme,
}

/// A finding is only exposed to clients once it has been published.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
#[serde(rename_all = "lowercase")]
pub enum FindingStatus {
    #[sea_orm(string_value = "draft")]
    Draft,
    #[sea_orm(string_value = "published")]
    Published,
}

impl Severity {
    /// Parses a caller-supplied severity. The accepted values in the message come
    /// from the enum, so they cannot drift from the ones actually accepted.
    ///
    /// # Errors
    /// When `value` is not one of the severities.
    pub fn parse(value: &str) -> std::result::Result<Self, String> {
        Self::iter().find(|s| s.to_value() == value).ok_or_else(|| {
            let accepted = Self::iter().map(|s| s.to_value()).collect::<Vec<_>>();
            format!("severity must be one of: {}", accepted.join(", "))
        })
    }
}

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
    /// Lists the findings that belong to a project.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn list_for_project(
        db: &DatabaseConnection,
        project_id: i64,
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
        self.status == FindingStatus::Published
    }
}
