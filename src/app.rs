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
        // Nothing may be checked here. loco's own `start` command calls
        // `create_app` directly, so this hook runs for the test harness and for
        // nothing else; what must stop a real start lives in the two hooks below.
        create_app::<Self, Migrator>(mode, environment, config).await
    }

    /// Runs for every command, as soon as there is a context.
    async fn after_context(ctx: AppContext) -> Result<AppContext> {
        // Fail closed: a malformed `settings:` block stops everything rather
        // than silently reverting to jwt mode (the least-safe default) in a
        // proxy-mode tenant — see `Settings::try_from_config`.
        crate::security::Settings::try_from_config(&ctx.config)?;
        Ok(ctx)
    }

    /// Runs when the app is about to serve.
    async fn before_run(ctx: &AppContext) -> Result<()> {
        let settings = crate::security::Settings::from_ctx(ctx);
        // Proxy mode trusts x-forwarded-* headers. If the defense-in-depth shared
        // secret isn't set the app leans on network isolation alone — say so
        // loudly instead of leaving it a silent no-op.
        if matches!(settings.auth_mode, crate::security::AuthMode::Proxy)
            && settings
                .proxy
                .shared_secret
                .as_deref()
                .is_none_or(str::is_empty)
        {
            tracing::warn!(
                "proxy auth mode without CASTLE_PROXY_SECRET: identity headers are trusted on \
                 network isolation alone — set a shared secret for defense-in-depth"
            );
        }
        // A logo that cannot be served is a configuration error, and one an
        // operator would otherwise only notice as a broken image in the header.
        if let Some(path) = settings.logo_path() {
            controllers::brand::load(path).await?;
        }
        Ok(())
    }

    fn routes(ctx: &AppContext) -> AppRoutes {
        // Honeypot instances present the same auth surface but capture what's
        // submitted instead of authenticating. Otherwise: proxy mode omits the
        // credential endpoints (oauth2-proxy authenticates); jwt mode keeps them.
        let settings = crate::security::Settings::from_ctx(ctx);
        let auth_routes = if settings.honeypot {
            controllers::auth::honeypot_routes()
        } else {
            match settings.auth_mode {
                crate::security::AuthMode::Jwt => controllers::auth::routes(),
                crate::security::AuthMode::Proxy => controllers::auth::proxy_routes(),
            }
        };
        AppRoutes::with_default_routes()
            .add_route(auth_routes)
            .add_route(controllers::projects::routes())
            .add_route(controllers::findings::routes())
            .add_route(controllers::comments::routes())
            .add_route(controllers::uploads::routes())
            .add_route(controllers::brand::routes())
    }
    /// Castle runs no background jobs — every operation it performs is
    /// request-scoped — but `Hooks` requires the registration point.
    async fn connect_workers(_ctx: &AppContext, _queue: &Queue) -> Result<()> {
        Ok(())
    }

    fn register_tasks(tasks: &mut Tasks) {
        // tasks-inject (do not remove)
        tasks.register(tasks::user_create::UserCreate);
        tasks.register(tasks::user_status::SetUserStatus);
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
