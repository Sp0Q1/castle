use loco_rs::prelude::*;

use crate::models::_entities::users;

pub struct AnonymizeUser;
#[async_trait]
impl Task for AnonymizeUser {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "user:anonymize".to_string(),
            detail: "Replace a person's name and address with a placeholder, disable the account and take it off every project. What they wrote stays. The account stays bound to the person, who is then this disabled account and not a new one; remove them from the IdP as well.\nUsage:\ncargo run task user:anonymize email:user@example.com".to_string(),
        }
    }
    async fn run(&self, app_context: &AppContext, vars: &task::Vars) -> Result<()> {
        let email = vars
            .cli_arg("email")
            .map_err(|_| Error::string("email is mandatory"))?;

        let user = users::Model::find_by_email(&app_context.db, &email.to_lowercase()).await?;
        let (user, removed) = user.anonymize(&app_context.db).await?;

        tracing::info!(
            user_pid = user.pid.to_string(),
            "account anonymized via task"
        );
        println!(
            "{email} is now `{}`: disabled and off {removed} project(s). What they wrote is kept.",
            user.name
        );
        Ok(())
    }
}
