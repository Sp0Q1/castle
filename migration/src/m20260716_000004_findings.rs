use loco_rs::schema::*;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        create_table(
            m,
            "findings",
            &[
                ("id", ColType::PkAuto),
                ("pid", ColType::Uuid),
                ("title", ColType::String),
                // The four required narrative sections of a report finding:
                ("description", ColType::Text),
                ("technical_description", ColType::Text),
                ("impact", ColType::Text),
                ("recommendation", ColType::Text),
                // info | low | medium | high | critical
                ("severity", ColType::StringWithDefault("medium".to_string())),
                // draft | published — clients only ever see "published" findings
                ("status", ColType::StringWithDefault("draft".to_string())),
            ],
            &[
                ("projects", ""),       // project_id INTEGER NOT NULL, FK -> projects(id)
                ("users", "author_id"), // author_id  INTEGER NOT NULL, FK -> users(id)
            ],
        )
        .await?;
        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        drop_table(m, "findings").await?;
        Ok(())
    }
}
