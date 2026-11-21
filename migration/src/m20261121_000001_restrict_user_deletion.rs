use sea_orm_migration::{
    prelude::*,
    sea_orm::{ConnectionTrait, DatabaseBackend, Statement},
};

#[derive(DeriveMigrationName)]
pub struct Migration;

// `create_table` makes every reference to a user `ON DELETE CASCADE`, so removing
// one row from `users` would take along that person's comments, the findings
// they wrote and every project they opened with all that is in it. Castle never
// deletes a user (an account is disabled, or anonymized by the `user:anonymize`
// task), and with this the database refuses to as long as anything points at
// them.
//
// The constraints are looked up rather than named: their names follow the
// framework's scheme at the time each database was created, and that scheme has
// changed.
//
// SQLite cannot change a constraint without rebuilding its table. It serves
// development and the test suite; every deployment runs Postgres.
async fn on_user_delete(m: &SchemaManager<'_>, action: &str) -> Result<(), DbErr> {
    if m.get_database_backend() != DatabaseBackend::Postgres {
        return Ok(());
    }
    let db = m.get_connection();
    let references = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT c.conrelid::regclass::text AS child, quote_ident(c.conname) AS name, \
                    quote_ident(a.attname) AS col \
             FROM pg_constraint c \
             JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = c.conkey[1] \
             WHERE c.contype = 'f' AND c.confrelid = 'users'::regclass",
        ))
        .await?;
    for row in references {
        let child: String = row.try_get("", "child")?;
        let name: String = row.try_get("", "name")?;
        let col: String = row.try_get("", "col")?;
        db.execute_unprepared(&format!(
            "ALTER TABLE {child} DROP CONSTRAINT {name}, ADD CONSTRAINT {name} \
             FOREIGN KEY ({col}) REFERENCES users (id) ON UPDATE CASCADE ON DELETE {action}"
        ))
        .await?;
    }
    Ok(())
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        on_user_delete(m, "RESTRICT").await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        on_user_delete(m, "CASCADE").await
    }
}
