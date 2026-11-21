use loco_rs::prelude::*;

use crate::models::{_entities::users, users::UserStatus};

pub struct SetUserStatus;
#[async_trait]
impl Task for SetUserStatus {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "user:status".to_string(),
            detail: "Disable an account, or enable it again. A disabled account is refused on every request, at once.\nUsage:\ncargo run task user:status email:user@example.com status:disabled".to_string(),
        }
    }
    async fn run(&self, app_context: &AppContext, vars: &task::Vars) -> Result<()> {
        let email = vars
            .cli_arg("email")
            .map_err(|_| Error::string("email is mandatory"))?;
        let status = match vars
            .cli_arg("status")
            .map_err(|_| Error::string("status is mandatory"))?
        {
            "disabled" => UserStatus::Disabled,
            "active" => UserStatus::Active,
            other => {
                return Err(Error::string(&format!(
                    "status must be `disabled` or `active`, not `{other}`"
                )))
            }
        };

        let mut user = users::Model::find_by_email(&app_context.db, &email.to_lowercase())
            .await?
            .into_active_model();
        user.status = ActiveValue::set(status);
        let user = user.update(&app_context.db).await?;

        tracing::info!(user_pid = user.pid.to_string(), status = ?user.status, "account status set via task");
        println!("{} is now {:?}", user.email, user.status);
        Ok(())
    }
}
