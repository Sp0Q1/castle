use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// `create_table` emits each foreign key constraint but no index on the column
// that carries it, so every one of these was a sequential scan: listing a
// project's findings, listing a finding's comments, and — from the other side —
// the scan the database runs across each child table when a parent row is
// deleted. `project_members` already has its own unique index and is not repeated.
const FOREIGN_KEYS: &[(&str, &str)] = &[
    ("comments", "finding_id"),
    ("comments", "user_id"),
    ("findings", "project_id"),
    ("findings", "author_id"),
    ("projects", "created_by"),
];

fn index_name(table: &str, column: &str) -> String {
    format!("idx-{table}-{column}")
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for (table, column) in FOREIGN_KEYS {
            m.create_index(
                Index::create()
                    .name(index_name(table, column))
                    .table(Alias::new(*table))
                    .col(Alias::new(*column))
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for (table, column) in FOREIGN_KEYS {
            m.drop_index(
                Index::drop()
                    .name(index_name(table, column))
                    .table(Alias::new(*table))
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }
}
