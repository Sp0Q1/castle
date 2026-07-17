use loco_rs::schema::*;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        create_table(
            m,
            "projects",
            &[
                ("id", ColType::PkAuto),
                ("pid", ColType::Uuid),
                ("name", ColType::String),
                ("description", ColType::TextNull),
                ("status", ColType::StringWithDefault("active".to_string())),
            ],
            &[
                // created_by INTEGER NOT NULL, FK -> users(id) (the manager who opened it)
                ("users", "created_by"),
            ],
        )
        .await?;
        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        drop_table(m, "projects").await?;
        Ok(())
    }
}
