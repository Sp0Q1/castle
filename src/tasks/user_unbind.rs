use loco_rs::prelude::*;

use crate::models::_entities::users;

pub struct UnbindUser;
#[async_trait]
impl Task for UnbindUser {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "user:unbind".to_string(),
            detail: "Forget which person an account is bound to. Whoever the identity provider next vouches for with the account's address takes it: for someone whose account there was made anew, or, with all:yes, after a move to another identity provider.\nUsage:\ncargo run task user:unbind email:user@example.com\ncargo run task user:unbind all:yes".to_string(),
        }
    }
    async fn run(&self, app_context: &AppContext, vars: &task::Vars) -> Result<()> {
        match (vars.cli_arg("email").ok(), vars.cli_arg("all").ok()) {
            (Some(email), None) => {
                let user = users::Model::find_by_email(&app_context.db, &email.to_lowercase())
                    .await?
                    .unbind(&app_context.db)
                    .await?;
                tracing::info!(user_pid = user.pid.to_string(), "account unbound via task");
                println!("{} is bound to nobody", user.email);
                Ok(())
            }
            (None, Some("yes")) => {
                let unbound = users::Model::unbind_all(&app_context.db).await?;
                tracing::info!(accounts = unbound, "all accounts unbound via task");
                println!("{unbound} accounts are bound to nobody");
                Ok(())
            }
            _ => Err(Error::string("give either email:<address> or all:yes")),
        }
    }
}
