use loco_rs::schema::*;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        // Comments let clients (and project staff) discuss a finding.
        create_table(
            m,
            "comments",
            &[
                ("id", ColType::PkAuto),
                ("body", ColType::Text),
            ],
            &[
                ("findings", ""), // finding_id INTEGER NOT NULL, FK -> findings(id)
                ("users", ""),    // user_id    INTEGER NOT NULL, FK -> users(id)
            ],
        )
        .await?;
        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        drop_table(m, "comments").await?;
        Ok(())
    }
}
