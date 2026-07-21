use async_trait::async_trait;
use chrono::{offset::Local, Duration};
use loco_rs::{auth::jwt, hash, prelude::*};
use serde::{Deserialize, Serialize};
use serde_json::Map;
use uuid::Uuid;

pub use super::_entities::users::{self, ActiveModel, Entity, Model};

use super::_entities::project_members;

pub const MAGIC_LINK_LENGTH: i8 = 32;
pub const MAGIC_LINK_EXPIRATION_MIN: i8 = 5;

#[derive(Debug, Deserialize, Serialize)]
pub struct LoginParams {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RegisterParams {
    pub email: String,
    pub password: String,
    pub name: String,
}

#[derive(Debug, Validate, Deserialize)]
pub struct Validator {
    #[validate(length(min = 2, message = "Name must be at least 2 characters long."))]
    pub name: String,
    #[validate(email(message = "invalid email"))]
    pub email: String,
}

impl Validatable for ActiveModel {
    fn validator(&self) -> Box<dyn Validate> {
        Box::new(Validator {
            name: self.name.as_ref().to_owned(),
            email: self.email.as_ref().to_owned(),
        })
    }
}

#[async_trait::async_trait]
impl ActiveModelBehavior for super::_entities::users::ActiveModel {
    async fn before_save<C>(self, _db: &C, insert: bool) -> Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        self.validate()?;
        if insert {
            let mut this = self;
            this.pid = ActiveValue::Set(Uuid::new_v4());
            this.api_key = ActiveValue::Set(format!("lo-{}", Uuid::new_v4()));
            Ok(this)
        } else {
            Ok(self)
        }
    }
}

#[async_trait]
impl Authenticable for Model {
    async fn find_by_api_key(db: &DatabaseConnection, api_key: &str) -> ModelResult<Self> {
        Self::find_by_api_key(db, api_key).await
    }

    async fn find_by_claims_key(db: &DatabaseConnection, claims_key: &str) -> ModelResult<Self> {
        Self::find_by_pid(db, claims_key).await
    }
}

impl Model {
    /// finds a user by the provided email
    ///
    /// # Errors
    ///
    /// When could not find user by the given token or DB query error
    pub async fn find_by_email(db: &DatabaseConnection, email: &str) -> ModelResult<Self> {
        let user = users::Entity::find()
            .filter(
                model::query::condition()
                    .eq(users::Column::Email, email)
                    .build(),
            )
            .one(db)
            .await?;
        user.ok_or_else(|| ModelError::EntityNotFound)
    }

    /// finds a user by the provided verification token
    ///
    /// # Errors
    ///
    /// When could not find user by the given token or DB query error
    pub async fn find_by_verification_token(
        db: &DatabaseConnection,
        token: &str,
    ) -> ModelResult<Self> {
        let user = users::Entity::find()
            .filter(
                model::query::condition()
                    .eq(users::Column::EmailVerificationToken, token)
                    .build(),
            )
            .one(db)
            .await?;
        user.ok_or_else(|| ModelError::EntityNotFound)
    }

    /// finds a user by the magic token and verify and token expiration
    ///
    /// # Errors
    ///
    /// When could not find user by the given token or DB query error ot token expired
    pub async fn find_by_magic_token(db: &DatabaseConnection, token: &str) -> ModelResult<Self> {
        let user = users::Entity::find()
            .filter(
                query::condition()
                    .eq(users::Column::MagicLinkToken, token)
                    .build(),
            )
            .one(db)
            .await?;

        let user = user.ok_or_else(|| ModelError::EntityNotFound)?;
        if let Some(expired_at) = user.magic_link_expiration {
            if expired_at >= Local::now() {
                Ok(user)
            } else {
                tracing::debug!(
                    user_pid = user.pid.to_string(),
                    token_expiration = expired_at.to_string(),
                    "magic token expired for the user."
                );
                Err(ModelError::msg("magic token expired"))
            }
        } else {
            tracing::error!(
                user_pid = user.pid.to_string(),
                "magic link expiration time not exists"
            );
            Err(ModelError::msg("expiration token not exists"))
        }
    }

    /// finds a user by the provided reset token
    ///
    /// # Errors
    ///
    /// When could not find user by the given token or DB query error
    pub async fn find_by_reset_token(db: &DatabaseConnection, token: &str) -> ModelResult<Self> {
        let user = users::Entity::find()
            .filter(
                model::query::condition()
                    .eq(users::Column::ResetToken, token)
                    .build(),
            )
            .one(db)
            .await?;
        user.ok_or_else(|| ModelError::EntityNotFound)
    }

    /// finds a user by the provided pid
    ///
    /// # Errors
    ///
    /// When could not find user  or DB query error
    pub async fn find_by_pid(db: &DatabaseConnection, pid: &str) -> ModelResult<Self> {
        let parse_uuid = Uuid::parse_str(pid).map_err(|e| ModelError::Any(e.into()))?;
        let user = users::Entity::find()
            .filter(
                model::query::condition()
                    .eq(users::Column::Pid, parse_uuid)
                    .build(),
            )
            .one(db)
            .await?;
        user.ok_or_else(|| ModelError::EntityNotFound)
    }

    /// finds a user by the provided api key
    ///
    /// # Errors
    ///
    /// When could not find user by the given token or DB query error
    pub async fn find_by_api_key(db: &DatabaseConnection, api_key: &str) -> ModelResult<Self> {
        let user = users::Entity::find()
            .filter(
                model::query::condition()
                    .eq(users::Column::ApiKey, api_key)
                    .build(),
            )
            .one(db)
            .await?;
        user.ok_or_else(|| ModelError::EntityNotFound)
    }

    /// Verifies whether the provided plain password matches the hashed password
    ///
    /// # Errors
    ///
    /// when could not verify password
    #[must_use]
    pub fn verify_password(&self, password: &str) -> bool {
        hash::verify_password(password, &self.password)
    }

    /// Asynchronously creates a user with a password and saves it to the
    /// database.
    ///
    /// # Errors
    ///
    /// When could not save the user into the DB
    pub async fn create_with_password(
        db: &DatabaseConnection,
        params: &RegisterParams,
    ) -> ModelResult<Self> {
        let txn = db.begin().await?;

        if users::Entity::find()
            .filter(
                model::query::condition()
                    .eq(users::Column::Email, &params.email)
                    .build(),
            )
            .one(&txn)
            .await?
            .is_some()
        {
            return Err(ModelError::EntityAlreadyExists {});
        }

        let password_hash =
            hash::hash_password(&params.password).map_err(|e| ModelError::Any(e.into()))?;
        let user = users::ActiveModel {
            email: ActiveValue::set(params.email.clone()),
            password: ActiveValue::set(password_hash),
            name: ActiveValue::set(params.name.clone()),
            ..Default::default()
        }
        .insert(&txn)
        .await?;

        txn.commit().await?;

        Ok(user)
    }

    /// Creates a JWT
    ///
    /// # Errors
    ///
    /// when could not convert user claims to jwt token
    pub fn generate_jwt(&self, secret: &str, expiration: u64) -> ModelResult<String> {
        jwt::JWT::new(secret)
            .generate_token(expiration, self.pid.to_string(), Map::new())
            .map_err(ModelError::from)
    }
}

impl ActiveModel {
    /// Sets the email verification information for the user and
    /// updates it in the database.
    ///
    /// This method is used to record the timestamp when the email verification
    /// was sent and generate a unique verification token for the user.
    ///
    /// # Errors
    ///
    /// when has DB query error
    pub async fn set_email_verification_sent(
        mut self,
        db: &DatabaseConnection,
    ) -> ModelResult<Model> {
        self.email_verification_sent_at = ActiveValue::set(Some(Local::now().into()));
        self.email_verification_token = ActiveValue::Set(Some(Uuid::new_v4().to_string()));
        self.update(db).await.map_err(ModelError::from)
    }

    /// Sets the information for a reset password request,
    /// generates a unique reset password token, and updates it in the
    /// database.
    ///
    /// This method records the timestamp when the reset password token is sent
    /// and generates a unique token for the user.
    ///
    /// # Arguments
    ///
    /// # Errors
    ///
    /// when has DB query error
    pub async fn set_forgot_password_sent(mut self, db: &DatabaseConnection) -> ModelResult<Model> {
        self.reset_sent_at = ActiveValue::set(Some(Local::now().into()));
        self.reset_token = ActiveValue::Set(Some(Uuid::new_v4().to_string()));
        self.update(db).await.map_err(ModelError::from)
    }

    /// Records the verification time when a user verifies their
    /// email and updates it in the database.
    ///
    /// This method sets the timestamp when the user successfully verifies their
    /// email.
    ///
    /// # Errors
    ///
    /// when has DB query error
    pub async fn verified(mut self, db: &DatabaseConnection) -> ModelResult<Model> {
        self.email_verified_at = ActiveValue::set(Some(Local::now().into()));
        self.update(db).await.map_err(ModelError::from)
    }

    /// Resets the current user password with a new password and
    /// updates it in the database.
    ///
    /// This method hashes the provided password and sets it as the new password
    /// for the user.
    ///
    /// # Errors
    ///
    /// when has DB query error or could not hashed the given password
    pub async fn reset_password(
        mut self,
        db: &DatabaseConnection,
        password: &str,
    ) -> ModelResult<Model> {
        self.password =
            ActiveValue::set(hash::hash_password(password).map_err(|e| ModelError::Any(e.into()))?);
        self.reset_token = ActiveValue::Set(None);
        self.reset_sent_at = ActiveValue::Set(None);
        self.update(db).await.map_err(ModelError::from)
    }

    /// Creates a magic link token for passwordless authentication.
    ///
    /// Generates a random token with a specified length and sets an expiration time
    /// for the magic link. This method is used to initiate the magic link authentication flow.
    ///
    /// # Errors
    /// - Returns an error if database update fails
    pub async fn create_magic_link(mut self, db: &DatabaseConnection) -> ModelResult<Model> {
        let random_str = hash::random_string(MAGIC_LINK_LENGTH as usize);
        let expired = Local::now() + Duration::minutes(MAGIC_LINK_EXPIRATION_MIN.into());

        self.magic_link_token = ActiveValue::set(Some(random_str));
        self.magic_link_expiration = ActiveValue::set(Some(expired.into()));
        self.update(db).await.map_err(ModelError::from)
    }

    /// Verifies and invalidates the magic link after successful authentication.
    ///
    /// Clears the magic link token and expiration time after the user has
    /// successfully authenticated using the magic link.
    ///
    /// # Errors
    /// - Returns an error if database update fails
    pub async fn clear_magic_link(mut self, db: &DatabaseConnection) -> ModelResult<Model> {
        self.magic_link_token = ActiveValue::set(None);
        self.magic_link_expiration = ActiveValue::set(None);
        self.update(db).await.map_err(ModelError::from)
    }
}

/// The role a user holds across the platform. Stored on `users.role` as a
/// lowercase string; use [`Model::role`] to read it in typed form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    /// Management: can open projects and onboard staff and clients.
    Manager,
    /// Consultant: can author and publish findings on their projects.
    Staff,
    /// Customer: can read published findings on their projects and comment.
    Client,
}

impl UserRole {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manager => "manager",
            Self::Staff => "staff",
            Self::Client => "client",
        }
    }

    /// Parses a stored role string, defaulting to [`Self::Staff`] for unknown
    /// values (matching the database default).
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "manager" => Self::Manager,
            "client" => Self::Client,
            _ => Self::Staff,
        }
    }
}

// role helpers
impl Model {
    /// The typed platform role for this user.
    #[must_use]
    pub fn role(&self) -> UserRole {
        UserRole::parse(&self.role)
    }

    #[must_use]
    pub fn is_manager(&self) -> bool {
        self.role() == UserRole::Manager
    }

    #[must_use]
    pub fn is_staff(&self) -> bool {
        self.role() == UserRole::Staff
    }

    #[must_use]
    pub fn is_client(&self) -> bool {
        self.role() == UserRole::Client
    }
}

// SSO / provisioning
impl Model {
    /// Reconciles an SSO identity (keyed by email) into a local user: creates it
    /// if new, otherwise syncs the display name, the IdP-authoritative role, and
    /// marks the account active. Called on every authenticated proxy request.
    ///
    /// # Errors
    /// When the DB query/update fails.
    pub async fn provision_from_sso(
        db: &DatabaseConnection,
        email: &str,
        name: Option<&str>,
        role: UserRole,
    ) -> ModelResult<Self> {
        match Self::find_by_email(db, email).await {
            Ok(user) => {
                let name_changed = name
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                    .is_some_and(|n| user.name != n);
                if user.role.as_str() == role.as_str()
                    && user.status.as_str() == "active"
                    && !name_changed
                {
                    return Ok(user);
                }
                let mut active = user.into_active_model();
                active.role = ActiveValue::set(role.as_str().to_string());
                active.status = ActiveValue::set("active".to_string());
                if let Some(n) = name.map(str::trim).filter(|n| !n.is_empty()) {
                    active.name = ActiveValue::set(n.to_string());
                }
                let updated = active.update(db).await.map_err(ModelError::from)?;
                // The IdP is authoritative for role. Authorization on findings keys
                // off `project_members.role`, so a demotion here must also drop any
                // membership that would still grant a higher capacity — otherwise a
                // user removed from the staff group keeps write access via the
                // stale membership row.
                Self::demote_stale_memberships(db, &updated).await?;
                Ok(updated)
            }
            Err(ModelError::EntityNotFound) => {
                Self::insert_shadow(db, email, name, role, "active").await
            }
            Err(e) => Err(e),
        }
    }

    /// Prevents a per-project membership from outranking the platform role.
    ///
    /// Called after an SSO reconciliation changes the role: when the IdP demotes
    /// someone to `client`, any `project_members` row still recording a higher
    /// capacity (e.g. `staff`) is downgraded, so findings authorization — which
    /// keys off the membership role — cannot be bypassed by a stale row.
    ///
    /// # Errors
    /// When the DB query/update fails.
    async fn demote_stale_memberships(db: &DatabaseConnection, user: &Self) -> ModelResult<()> {
        if user.role.as_str() != UserRole::Client.as_str() {
            return Ok(());
        }
        let stale = project_members::Entity::find()
            .filter(project_members::Column::UserId.eq(user.id))
            .filter(project_members::Column::Role.ne(UserRole::Client.as_str()))
            .all(db)
            .await
            .map_err(ModelError::from)?;
        for membership in stale {
            let mut active = membership.into_active_model();
            active.role = ActiveValue::set(UserRole::Client.as_str().to_string());
            active.update(db).await.map_err(ModelError::from)?;
        }
        Ok(())
    }

    /// Finds a user by email, or creates an "invited" placeholder so a manager
    /// can onboard someone who has not signed in yet (reconciled on first login).
    ///
    /// # Errors
    /// When the DB query/insert fails.
    pub async fn find_or_invite(db: &DatabaseConnection, email: &str) -> ModelResult<Self> {
        match Self::find_by_email(db, email).await {
            Ok(user) => Ok(user),
            Err(ModelError::EntityNotFound) => {
                Self::insert_shadow(db, email, None, UserRole::Client, "invited").await
            }
            Err(e) => Err(e),
        }
    }

    async fn insert_shadow(
        db: &DatabaseConnection,
        email: &str,
        name: Option<&str>,
        role: UserRole,
        status: &str,
    ) -> ModelResult<Self> {
        // No usable password: these users authenticate via the IdP/proxy, never here.
        let random = hash::hash_password(&Uuid::new_v4().to_string())
            .map_err(|e| ModelError::Any(e.into()))?;
        let display = name
            .map(str::trim)
            .filter(|n| n.len() >= 2)
            .unwrap_or(email)
            .to_string();
        let user = users::ActiveModel {
            email: ActiveValue::set(email.to_string()),
            name: ActiveValue::set(display),
            password: ActiveValue::set(random),
            role: ActiveValue::set(role.as_str().to_string()),
            status: ActiveValue::set(status.to_string()),
            ..Default::default()
        }
        .insert(db)
        .await?;
        Ok(user)
    }
}
