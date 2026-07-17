use loco_rs::schema::*;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        // Role gates what a user may do across the app:
        //   "manager" -> create projects and onboard staff/clients
        //   "staff"   -> write & publish findings on their projects
        //   "client"  -> read published findings on their projects and comment
        add_column(
            m,
            "users",
            "role",
            ColType::StringWithDefault("staff".to_string()),
        )
        .await?;
        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        remove_column(m, "users", "role").await?;
        Ok(())
    }
}
