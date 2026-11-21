use loco_rs::schema::*;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const INDEX: &str = "idx-users-subject";

// `subject` is the identity provider's own id for the person behind an
// account. It is empty for an invitation and for an account nobody has signed
// in to since the column exists; the first sign-in with the account's address
// fills it in. No two accounts share one.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        add_column(m, "users", "subject", ColType::StringNull).await?;
        m.create_index(
            Index::create()
                .name(INDEX)
                .table(Alias::new("users"))
                .col(Alias::new("subject"))
                .unique()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_index(
            Index::drop()
                .name(INDEX)
                .table(Alias::new("users"))
                .to_owned(),
        )
        .await?;
        remove_column(m, "users", "subject").await
    }
}
