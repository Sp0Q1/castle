use loco_rs::schema::*;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        // A free-text classification for the finding (e.g. "SQL Injection",
        // "Broken Access Control"). The UI suggests values already used on the
        // same project so teams can keep them consistent.
        add_column(
            m,
            "findings",
            "finding_type",
            ColType::StringWithDefault(String::new()),
        )
        .await?;
        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        remove_column(m, "findings", "finding_type").await?;
        Ok(())
    }
}
