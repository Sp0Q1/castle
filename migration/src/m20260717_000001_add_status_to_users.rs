use loco_rs::schema::*;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        // "active" once the user has signed in; "invited" for accounts a manager
        // pre-created by onboarding an email that has not signed in yet (they are
        // reconciled on first SSO login, matched by email).
        add_column(
            m,
            "users",
            "status",
            ColType::StringWithDefault("active".to_string()),
        )
        .await?;
        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        remove_column(m, "users", "status").await?;
        Ok(())
    }
}
