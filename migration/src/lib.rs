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
mod m20260919_000001_index_foreign_keys;
mod m20261121_000001_restrict_user_deletion;
mod m20261121_000002_memberships_are_staff_or_client;
mod m20261121_000003_users_subject;

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
            Box::new(m20260919_000001_index_foreign_keys::Migration),
            Box::new(m20261121_000001_restrict_user_deletion::Migration),
            Box::new(m20261121_000002_memberships_are_staff_or_client::Migration),
            Box::new(m20261121_000003_users_subject::Migration),
            // inject-above (do not remove this comment)
        ]
    }
}
