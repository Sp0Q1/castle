use std::path::Path;
use async_trait::async_trait;
use loco_rs::{
    app::{AppContext, Hooks, Initializer},
    bgworker::{
        BackgroundWorker,
        Queue},
    boot::{create_app, BootResult, StartMode},
    config::Config,
    controller::AppRoutes,
    db::{self, truncate_table},
    environment::Environment,
    task::Tasks,
    Result,
};
use migration::Migrator;

#[allow(unused_imports)]
use crate::{
    controllers ,tasks, initializers
    , models::_entities::{comments, findings, project_members, projects, users}
    , workers::downloader::DownloadWorker
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

    async fn boot(mode: StartMode, environment: &Environment, config: Config) -> Result<BootResult> {
        create_app::<Self, Migrator>(mode, environment, config).await
        
    }

    async fn initializers(_ctx: &AppContext) -> Result<Vec<Box<dyn Initializer>>> {
        Ok(vec![Box::new(initializers::view_engine::ViewEngineInitializer)])
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
    }
    async fn connect_workers(ctx: &AppContext, queue: &Queue) -> Result<()> {
        queue.register(DownloadWorker::build(ctx)).await?;
        Ok(())
    }

    #[allow(unused_variables)]
    fn register_tasks(tasks: &mut Tasks) {
        // tasks-inject (do not remove)
        tasks.register(tasks::user_create::UserCreate); 
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
        db::seed::<users::ActiveModel>(&ctx.db, &base.join("users.yaml").display().to_string()).await?;
        Ok(())
    }
}
