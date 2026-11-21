use async_trait::async_trait;
use loco_rs::{
    app::{AppContext, Hooks},
    bgworker::Queue,
    boot::{create_app, BootResult, StartMode},
    config::Config,
    controller::AppRoutes,
    db::{self, truncate_table},
    environment::Environment,
    task::Tasks,
    Result,
};
use migration::Migrator;
use std::path::Path;

use crate::{
    controllers,
    models::_entities::{comments, findings, project_members, projects, users},
    tasks,
};

pub struct App;
#[async_trait]
impl Hooks for App {
    fn app_name() -> &'static str {
        env!("CARGO_CRATE_NAME")
    }

    fn app_version() -> String {
        format!(
            "{} ({})",
            env!("CARGO_PKG_VERSION"),
            option_env!("BUILD_SHA")
                .or(option_env!("GITHUB_SHA"))
                .unwrap_or("dev")
        )
    }

    async fn boot(
        mode: StartMode,
        environment: &Environment,
        config: Config,
    ) -> Result<BootResult> {
        // loco's `start` command calls `create_app` directly; only the test
        // harness comes through here. What must stop a start lives in the two
        // hooks below.
        create_app::<Self, Migrator>(mode, environment, config).await
    }

    /// Runs for every command, as soon as there is a context.
    async fn after_context(ctx: AppContext) -> Result<AppContext> {
        // Fail closed: a malformed `settings:` block, or proxy mode without the
        // proxy's secret, stops everything rather than silently reverting to
        // jwt mode or believing whoever reaches the port — see
        // `Settings::try_from_config`.
        let settings = crate::security::Settings::try_from_config(&ctx.config)?;
        ctx.shared_store.insert(settings);
        Ok(ctx)
    }

    /// Runs when the app is about to serve.
    async fn before_run(ctx: &AppContext) -> Result<()> {
        let settings = crate::security::Settings::from_ctx(ctx);
        // A logo that cannot be served is a configuration error, and one an
        // operator would otherwise only notice as a broken image in the header.
        if let Some(path) = settings.logo_path() {
            controllers::brand::load(path).await?;
        }
        Ok(())
    }

    fn routes(ctx: &AppContext) -> AppRoutes {
        // Proxy mode has no credential endpoints: oauth2-proxy signs people in.
        // jwt mode keeps them.
        let settings = crate::security::Settings::from_ctx(ctx);
        let auth_routes = match settings.auth_mode {
            crate::security::AuthMode::Jwt => controllers::auth::routes(),
            crate::security::AuthMode::Proxy => controllers::auth::proxy_routes(),
        };
        AppRoutes::with_default_routes()
            .add_route(auth_routes)
            .add_route(controllers::projects::routes())
            .add_route(controllers::findings::routes())
            .add_route(controllers::comments::routes())
            .add_route(controllers::uploads::routes())
            .add_route(controllers::brand::routes())
            .add_route(controllers::people::routes())
    }
    /// Castle runs no background jobs — every operation it performs is
    /// request-scoped — but `Hooks` requires the registration point.
    async fn connect_workers(_ctx: &AppContext, _queue: &Queue) -> Result<()> {
        Ok(())
    }

    fn register_tasks(tasks: &mut Tasks) {
        // tasks-inject (do not remove)
        tasks.register(tasks::user_anonymize::AnonymizeUser);
        tasks.register(tasks::user_create::UserCreate);
        tasks.register(tasks::user_status::SetUserStatus);
        tasks.register(tasks::user_unbind::UnbindUser);
    }
    async fn truncate(ctx: &AppContext) -> Result<()> {
        // Truncate children before parents so foreign keys stay satisfied.
        truncate_table(&ctx.db, comments::Entity).await?;
        truncate_table(&ctx.db, findings::Entity).await?;
        truncate_table(&ctx.db, project_members::Entity).await?;
        truncate_table(&ctx.db, projects::Entity).await?;
        truncate_table(&ctx.db, users::Entity).await?;
        Ok(())
    }
    async fn seed(ctx: &AppContext, base: &Path) -> Result<()> {
        db::seed::<users::ActiveModel>(&ctx.db, &base.join("users.yaml").display().to_string())
            .await?;
        Ok(())
    }
}
