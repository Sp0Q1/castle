use loco_rs::schema::*;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        // The onboarding join: which users (staff and clients) belong to a project.
        // `role` records the capacity in which they were onboarded ("staff" | "client").
        create_table(
            m,
            "project_members",
            &[
                ("id", ColType::PkAuto),
                ("role", ColType::StringWithDefault("staff".to_string())),
            ],
            &[
                ("projects", ""), // project_id INTEGER NOT NULL, FK -> projects(id)
                ("users", ""),    // user_id    INTEGER NOT NULL, FK -> users(id)
            ],
        )
        .await?;

        // A user can be onboarded to a given project only once.
        m.create_index(
            Index::create()
                .name("idx-project_members-project-user")
                .table(Alias::new("project_members"))
                .col(Alias::new("project_id"))
                .col(Alias::new("user_id"))
                .unique()
                .to_owned(),
        )
        .await?;
        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        drop_table(m, "project_members").await?;
        Ok(())
    }
}
