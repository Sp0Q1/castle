//! Authentication mode configuration and the [`CurrentUser`] request extractor.
//!
//! Castle supports two modes, selected by `settings.auth_mode` in the config:
//!
//! - `jwt`   — loco's built-in JWT (login/register). Used for local dev & tests.
//! - `proxy` — the app runs behind **oauth2-proxy**, which performs the full
//!   OIDC dance with Keycloak and injects identity headers. Castle trusts those
//!   headers and never handles tokens or secrets itself.
//!
//! SECURITY: in `proxy` mode the identity headers are trusted unconditionally,
//! so the app MUST be reachable only through the proxy (private bind + network
//! policy). The optional `shared_secret` header is defense-in-depth, not a
//! substitute for network isolation.
use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use loco_rs::prelude::*;
use serde::Deserialize;

use crate::models::_entities::users;
use crate::models::users::UserRole;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    #[default]
    Jwt,
    Proxy,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ProxySettings {
    /// Header carrying the authenticated email (oauth2-proxy: `x-forwarded-email`
    /// in full reverse-proxy mode, or `x-auth-request-email`).
    pub email_header: String,
    /// Header carrying the display name / preferred username.
    pub name_header: String,
    /// Header carrying the user's groups (comma/space separated).
    pub groups_header: String,
    /// Optional shared secret the proxy injects; when set, requests without a
    /// matching value are rejected (defense-in-depth against direct access).
    pub shared_secret_header: Option<String>,
    pub shared_secret: Option<String>,
    /// Keycloak group (leaf name) that grants each platform role.
    pub manager_group: String,
    pub staff_group: String,
    pub client_group: String,
    /// Where the SPA sends the browser on sign-out. For a full RP-initiated
    /// logout point this at oauth2-proxy's sign-out chained to the IdP's
    /// end-session endpoint, e.g.
    /// `/oauth2/sign_out?rd=<url-encoded Keycloak .../logout?post_logout_redirect_uri=…&client_id=castle>`.
    pub logout_url: Option<String>,
}

impl Default for ProxySettings {
    fn default() -> Self {
        Self {
            email_header: "x-forwarded-email".to_string(),
            name_header: "x-forwarded-preferred-username".to_string(),
            groups_header: "x-forwarded-groups".to_string(),
            shared_secret_header: None,
            shared_secret: None,
            manager_group: "castle-managers".to_string(),
            staff_group: "castle-staff".to_string(),
            client_group: "castle-clients".to_string(),
            logout_url: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    pub auth_mode: AuthMode,
    pub proxy: ProxySettings,
}

impl Settings {
    #[must_use]
    pub fn from_ctx(ctx: &AppContext) -> Self {
        ctx.config
            .settings
            .as_ref()
            .and_then(|v| serde_json::from_value::<Self>(v.clone()).ok())
            .unwrap_or_default()
    }
}

impl ProxySettings {
    /// Maps the groups header to a platform role. Exact leaf-name match (a
    /// leading `/path/` from Keycloak is stripped); highest privilege wins;
    /// unknown/empty defaults to least privilege (client).
    #[must_use]
    pub fn role_from_groups(&self, groups: &str) -> UserRole {
        let has = |target: &str| {
            groups.split([',', ' ', ';']).map(str::trim).any(|g| {
                let leaf = g.rsplit('/').next().unwrap_or(g);
                leaf == target
            })
        };
        if has(&self.manager_group) {
            UserRole::Manager
        } else if has(&self.staff_group) {
            UserRole::Staff
        } else {
            UserRole::Client
        }
    }
}

/// The authenticated user for a request, resolved per the configured auth mode.
/// Use as a handler argument: `CurrentUser(user): CurrentUser`.
pub struct CurrentUser(pub users::Model);

impl<S> FromRequestParts<S> for CurrentUser
where
    AppContext: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Error> {
        let ctx = AppContext::from_ref(state);
        let settings = Settings::from_ctx(&ctx);
        match settings.auth_mode {
            AuthMode::Jwt => {
                let jwt = auth::JWT::from_request_parts(parts, state).await?;
                let user = users::Model::find_by_pid(&ctx.db, &jwt.claims.pid)
                    .await
                    .map_err(|_| Error::Unauthorized("user not found".to_string()))?;
                Ok(Self(user))
            }
            AuthMode::Proxy => {
                let proxy = &settings.proxy;
                if let (Some(name), Some(expected)) =
                    (proxy.shared_secret_header.as_ref(), proxy.shared_secret.as_ref())
                {
                    // An empty configured secret means "not set" — don't enforce.
                    if !expected.is_empty() {
                        let got = parts.headers.get(name).and_then(|v| v.to_str().ok());
                        if got != Some(expected.as_str()) {
                            return Err(Error::Unauthorized(
                                "request did not arrive through the authenticating proxy"
                                    .to_string(),
                            ));
                        }
                    }
                }
                let read = |name: &str| {
                    parts
                        .headers
                        .get(name)
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string)
                };
                let email = read(&proxy.email_header)
                    .filter(|e| !e.is_empty())
                    .ok_or_else(|| {
                        Error::Unauthorized("no authenticated identity from proxy".to_string())
                    })?;
                let name = read(&proxy.name_header);
                let role = read(&proxy.groups_header)
                    .map_or(UserRole::Client, |g| proxy.role_from_groups(&g));
                let user = users::Model::provision_from_sso(
                    &ctx.db,
                    &email.to_lowercase(),
                    name.as_deref(),
                    role,
                )
                .await?;
                Ok(Self(user))
            }
        }
    }
}
