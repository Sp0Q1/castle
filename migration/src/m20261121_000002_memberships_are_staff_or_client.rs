use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// A manager is on no project: the role opens all of them. The rows that once
// named the manager who opened a project said nothing `projects.created_by`
// does not say, and they go, so that a membership is a staff or a client
// capacity and nothing else.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.exec_stmt(
            Query::delete()
                .from_table(Alias::new("project_members"))
                .and_where(Expr::col(Alias::new("role")).eq("manager"))
                .to_owned(),
        )
        .await
    }

    async fn down(&self, _m: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
