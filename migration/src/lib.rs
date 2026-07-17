#![allow(elided_lifetimes_in_paths)]
#![allow(clippy::wildcard_imports)]
pub use sea_orm_migration::prelude::*;

mod m20220101_000001_users;
mod m20260716_000001_add_role_to_users;
mod m20260716_000002_projects;
mod m20260716_000003_project_members;
mod m20260716_000004_findings;
mod m20260716_000005_comments;
mod m20260716_000006_add_type_to_findings;
mod m20260717_000001_add_status_to_users;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20220101_000001_users::Migration),
            Box::new(m20260716_000001_add_role_to_users::Migration),
            Box::new(m20260716_000002_projects::Migration),
            Box::new(m20260716_000003_project_members::Migration),
            Box::new(m20260716_000004_findings::Migration),
            Box::new(m20260716_000005_comments::Migration),
            Box::new(m20260716_000006_add_type_to_findings::Migration),
            Box::new(m20260717_000001_add_status_to_users::Migration),
            // inject-above (do not remove this comment)
        ]
    }
}
