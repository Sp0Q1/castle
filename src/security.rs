//! Authentication mode configuration and the [`CurrentUser`] request extractor.
//!
//! Castle supports two modes, selected by `settings.auth_mode` in the config:
//!
//! - `jwt`   — loco's built-in JWT (login/register). Used for local dev & tests.
//! - `proxy` — the app runs behind **oauth2-proxy**, which performs the full
//!   OIDC dance with Keycloak and injects identity headers. Castle trusts those
//!   headers and never handles tokens or secrets itself.
//!
//! SECURITY: in `proxy` mode the identity headers are believed, so they are
//! only read from a request that proves it came through this instance's own
//! proxy: it carries the secret the two share (`settings.proxy.secret`), which
//! oauth2-proxy sends as the password of a Basic `Authorization` header. Proxy
//! mode does not start without that secret. The network the app sits on, which
//! only its proxy joins, is the second lock and not the only one.
//!
//! What the proxy says is read once, by [`ProxyIdentity::recognize`], and
//! strictly: a header sent twice, an address that is none, or a group name
//! that is only nearly right makes nobody, or a client, never more.
use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use axum::http::HeaderMap;
use base64::prelude::{Engine as _, BASE64_STANDARD};
use loco_rs::prelude::*;
use serde::Deserialize;
use validator::ValidateEmail;

use crate::models::_entities::users;
use crate::models::users::{UserRole, UserStatus};

/// How people sign in. There is no default: a configuration that does not say
/// is not started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    Jwt,
    Proxy,
}

/// The secret an instance shares with its own proxy. There is no empty one,
/// and `Debug` shows nothing of it.
#[derive(Clone)]
pub struct ProxySecret(String);

impl ProxySecret {
    /// An empty value means "not set", so a configuration can always pass the
    /// variable.
    fn or_unset<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Option<Self>, D::Error> {
        let value = Option::<String>::deserialize(deserializer)?;
        Ok(value.filter(|value| !value.is_empty()).map(Self))
    }

    /// Whether `presented` is this secret. Constant-time, so a byte-by-byte
    /// timing oracle can't recover it across many requests. Length may leak:
    /// it isn't secret; only the bytes are.
    #[must_use]
    pub fn is(&self, presented: &[u8]) -> bool {
        let expected = self.0.as_bytes();
        presented.len() == expected.len()
            && presented
                .iter()
                .zip(expected)
                .fold(0u8, |acc, (x, y)| acc | (x ^ y))
                == 0
    }
}

impl std::fmt::Debug for ProxySecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProxySecret(set)")
    }
}

/// A group's name as it can stand in the groups header: not empty, and without
/// the comma that separates one name from the next.
fn group_name<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<String, D::Error> {
    let name = String::deserialize(deserializer)?;
    if name.is_empty() || name.contains(',') {
        return Err(serde::de::Error::custom(format!(
            "a group name is not empty and has no comma, got {name:?}"
        )));
    }
    Ok(name)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProxySettings {
    /// Header carrying the authenticated email (oauth2-proxy: `x-forwarded-email`
    /// in full reverse-proxy mode, or `x-auth-request-email`).
    pub email_header: String,
    /// Header carrying the display name / preferred username.
    pub name_header: String,
    /// Header carrying the user's groups, as oauth2-proxy writes them: names
    /// separated by commas.
    pub groups_header: String,
    /// The secret this instance shares with its own proxy. A request's identity
    /// headers are read only when it carries this secret.
    #[serde(deserialize_with = "ProxySecret::or_unset")]
    pub secret: Option<ProxySecret>,
    /// The Keycloak group that gives each role, by its exact name.
    #[serde(deserialize_with = "group_name")]
    pub manager_group: String,
    #[serde(deserialize_with = "group_name")]
    pub staff_group: String,
    #[serde(deserialize_with = "group_name")]
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
            secret: None,
            manager_group: "castle-managers".to_string(),
            staff_group: "castle-staff".to_string(),
            client_group: "castle-clients".to_string(),
            logout_url: None,
        }
    }
}

/// The `settings:` block of the configuration, read once when castle starts
/// ([`Settings::try_from_config`]) and kept for every request. A key it does
/// not know is an error, so a setting that was renamed or mistyped stops the
/// start instead of being left out.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub auth_mode: AuthMode,
    #[serde(default)]
    pub proxy: ProxySettings,
    /// Decoy mode: the auth endpoints capture submitted credentials + attacker
    /// metadata and return believable responses, without ever touching a real
    /// DB. Set on canary/honeypot instances only.
    #[serde(default)]
    pub honeypot: bool,
    /// Path of an image that replaces the built-in brand in the portal's header
    /// (see `controllers::brand`). Empty or absent: the built-in brand.
    #[serde(default)]
    pub logo: Option<String>,
}

impl Settings {
    /// The configured logo, if there is one. An empty value means "not set", so
    /// a deployment can always pass the variable.
    #[must_use]
    pub fn logo_path(&self) -> Option<&str> {
        self.logo.as_deref().filter(|path| !path.is_empty())
    }

    /// The settings castle started with. `App::after_context` reads them once
    /// and keeps them; a context that never went through it reads them here,
    /// by the same rules.
    ///
    /// # Panics
    /// When there is nothing kept and the configuration is not one castle
    /// starts with, which `after_context` rules out for anything that serves.
    #[must_use]
    pub fn from_ctx(ctx: &AppContext) -> Self {
        ctx.shared_store.get::<Self>().unwrap_or_else(|| {
            Self::try_from_config(&ctx.config).expect("the settings are checked at start")
        })
    }

    /// Reads the `settings:` block, strictly.
    ///
    /// Called when castle starts, so that a value it cannot read **stops the
    /// app** instead of leaving a default in its place: a mode that is none, a
    /// key it does not know, a group name that could never match, or proxy
    /// mode without the secret that tells its own proxy from anything else
    /// that reaches the port.
    ///
    /// # Errors
    /// When there is no `settings:` block or it is not one this reads.
    pub fn try_from_config(config: &loco_rs::config::Config) -> Result<Self> {
        let invalid = |why: &dyn std::fmt::Display| {
            Error::Message(format!("invalid `settings:` config: {why}"))
        };
        let block = config
            .settings
            .as_ref()
            .ok_or_else(|| invalid(&"there is no `settings:` block"))?;
        Self::parse(block).map_err(|why| invalid(&why))
    }

    fn parse(block: &serde_json::Value) -> std::result::Result<Self, String> {
        let settings = Self::deserialize(block).map_err(|e| e.to_string())?;
        if settings.auth_mode == AuthMode::Proxy && settings.proxy.secret.is_none() {
            return Err(
                "proxy mode needs `proxy.secret` (CASTLE_PROXY_SECRET), the secret \
                        this instance shares with its proxy"
                    .to_string(),
            );
        }
        Ok(settings)
    }
}

/// 403 for an authenticated user who lacks the required role — distinct from the
/// 401 an *unauthenticated* request gets, and from the 404 used to hide a
/// resource a non-member must not even learn exists.
#[must_use]
pub fn forbidden(message: &str) -> Error {
    Error::CustomError(
        axum::http::StatusCode::FORBIDDEN,
        loco_rs::controller::ErrorDetail::new("forbidden", message),
    )
}

/// Whether `headers` carry the proxy's secret: exactly one `Authorization`
/// header, of the Basic scheme, whose password is `secret`. That is what
/// oauth2-proxy sends upstream with `--pass-basic-auth` and a basic-auth
/// password, and it replaces whatever the browser sent under that name.
#[must_use]
fn from_own_proxy(headers: &HeaderMap, secret: &ProxySecret) -> bool {
    let mut sent = headers.get_all(axum::http::header::AUTHORIZATION).iter();
    let (Some(value), None) = (sent.next(), sent.next()) else {
        return false;
    };
    value
        .as_bytes()
        .strip_prefix(b"Basic ")
        .and_then(|encoded| BASE64_STANDARD.decode(encoded).ok())
        .is_some_and(|credentials| {
            credentials
                .splitn(2, |byte| *byte == b':')
                .nth(1)
                .is_some_and(|password| secret.is(password))
        })
}

impl ProxySettings {
    /// The role a groups header gives: the highest among its names, and client
    /// when none of them gives one. A name counts when it is exactly a
    /// configured one, so neither a path nor a longer name that contains it
    /// does.
    #[must_use]
    pub fn role_of(&self, groups: &str) -> UserRole {
        let has = |group: &str| groups.split(',').any(|name| name == group);
        if has(&self.manager_group) {
            UserRole::Manager
        } else if has(&self.staff_group) {
            UserRole::Staff
        } else {
            UserRole::Client
        }
    }
}

/// The one value of a header. `Some(None)` when it is not sent; `None` when it
/// is sent more than once or is not UTF-8, which nothing here guesses at.
fn sent_once<'a>(headers: &'a HeaderMap, name: &str) -> Option<Option<&'a str>> {
    let mut values = headers.get_all(name).iter();
    match (values.next(), values.next()) {
        (None, _) => Some(None),
        (Some(value), None) => std::str::from_utf8(value.as_bytes()).ok().map(Some),
        (Some(_), Some(_)) => None,
    }
}

/// Who the proxy says is asking: its identity headers, read once. Nothing else
/// in the app looks at them.
#[derive(Debug, PartialEq, Eq)]
struct ProxyIdentity {
    email: String,
    name: Option<String>,
    role: UserRole,
}

impl ProxyIdentity {
    /// An address is at most this long (RFC 5321).
    const EMAIL_MAX: usize = 254;

    /// Reads the identity headers of a request that came through the proxy.
    /// The address has to be there and be one; it is taken in lower case. The
    /// name is optional. Without a groups header the person is a client.
    /// `None` when a header does not fit: the request is then nobody's, and
    /// nothing is made of the headers that did.
    fn recognize(headers: &HeaderMap, proxy: &ProxySettings) -> Option<Self> {
        let email = sent_once(headers, &proxy.email_header)??.to_lowercase();
        if email.len() > Self::EMAIL_MAX || !email.validate_email() {
            return None;
        }
        let name = sent_once(headers, &proxy.name_header)?.map(str::to_string);
        let role = sent_once(headers, &proxy.groups_header)?
            .map_or(UserRole::Client, |groups| proxy.role_of(groups));
        Some(Self { email, name, role })
    }
}

/// Whether a request comes from one of this site's own pages, by the browser's
/// word, which a page can neither set nor change: `Sec-Fetch-Site:
/// same-origin`, or, from a browser that does not send that header, an
/// `Origin` that is this site's.
fn from_this_site(headers: &HeaderMap, site: &str) -> bool {
    match sent_once(headers, "sec-fetch-site") {
        Some(Some(relation)) => relation == "same-origin",
        Some(None) => sent_once(headers, "origin")
            .flatten()
            .is_some_and(|origin| origin == site.trim_end_matches('/')),
        None => false,
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
        let user = match settings.auth_mode {
            AuthMode::Jwt => {
                let jwt = auth::JWT::from_request_parts(parts, state).await?;
                users::Model::find_by_pid(&ctx.db, &jwt.claims.pid)
                    .await
                    .map_err(|_| Error::Unauthorized("user not found".to_string()))?
            }
            AuthMode::Proxy => {
                let proxy = &settings.proxy;
                // Checked before a single identity header is read. An instance
                // without a secret does not start in this mode, and here it
                // would believe nobody.
                let through_proxy = proxy
                    .secret
                    .as_ref()
                    .is_some_and(|secret| from_own_proxy(&parts.headers, secret));
                if !through_proxy {
                    return Err(Error::Unauthorized(
                        "request did not arrive through the authenticating proxy".to_string(),
                    ));
                }
                // A request that changes something has to come from this
                // site's own pages: the session is a cookie, and a browser
                // sends it along with requests that other sites start.
                if !parts.method.is_safe()
                    && !from_this_site(&parts.headers, &ctx.config.server.host)
                {
                    return Err(forbidden("this request did not come from the portal"));
                }
                let who = ProxyIdentity::recognize(&parts.headers, proxy).ok_or_else(|| {
                    Error::Unauthorized("no authenticated identity from proxy".to_string())
                })?;
                users::Model::provision_from_sso(&ctx.db, &who.email, who.name.as_deref(), who.role)
                    .await?
            }
        };
        // Checked here, on every request and in both modes, so disabling an
        // account takes effect at once: neither a proxy session nor a token that
        // is still valid gets past it.
        if user.status == UserStatus::Disabled {
            return Err(forbidden("this account is disabled"));
        }
        Ok(Self(user))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_settings_are_read_strictly() {
        let read = |block: serde_json::Value| Settings::parse(&block);
        assert!(read(json!({ "auth_mode": "jwt" })).is_ok());
        assert!(read(json!({ "auth_mode": "proxy", "proxy": { "secret": "s" } })).is_ok());

        let refused = [
            (json!({}), "no mode"),
            (json!({ "auth_mode": "Proxy" }), "a mode that is none"),
            (
                json!({ "auth_mode": "proxy" }),
                "proxy mode without a secret",
            ),
            (
                json!({ "auth_mode": "proxy", "proxy": { "secret": "" } }),
                "an empty secret",
            ),
            (
                json!({ "auth_mode": "jwt", "honeypots": true }),
                "a key that is not one",
            ),
            (
                json!({ "auth_mode": "proxy", "proxy": { "secret": "s", "shared_secret": "old" } }),
                "a proxy key that is not one",
            ),
            (
                json!({ "auth_mode": "proxy", "proxy": { "secret": "s", "manager_group": "" } }),
                "a group without a name",
            ),
            (
                json!({ "auth_mode": "proxy", "proxy": { "secret": "s", "staff_group": "a,b" } }),
                "a group name with a comma",
            ),
            (
                json!({ "auth_mode": "jwt", "honeypot": 0 }),
                "a switch that is no boolean",
            ),
        ];
        for (block, what) in refused {
            assert!(read(block).is_err(), "{what} was accepted");
        }
    }

    #[test]
    fn a_group_counts_by_its_exact_name() {
        let proxy = ProxySettings::default();
        for (groups, role) in [
            ("castle-managers", UserRole::Manager),
            ("castle-clients,castle-managers", UserRole::Manager),
            ("castle-staff,castle-clients", UserRole::Staff),
            ("castle-clients", UserRole::Client),
            ("", UserRole::Client),
            ("castle-managers ", UserRole::Client),
            ("/castle-managers", UserRole::Client),
            ("x/castle-managers", UserRole::Client),
            ("castle-managers;castle-staff", UserRole::Client),
            ("castle-clients, castle-staff", UserRole::Client),
        ] {
            assert_eq!(proxy.role_of(groups), role, "{groups:?}");
        }
    }

    #[test]
    fn a_changing_request_is_this_sites_by_the_browsers_word() {
        let with = |pairs: &[(&'static str, &'static str)]| {
            let mut headers = HeaderMap::new();
            for (name, value) in pairs {
                headers.append(*name, value.parse().expect("a header value"));
            }
            from_this_site(&headers, "https://portal.example/")
        };
        assert!(with(&[("sec-fetch-site", "same-origin")]));
        assert!(with(&[("origin", "https://portal.example")]));
        assert!(!with(&[]));
        assert!(!with(&[("sec-fetch-site", "same-site")]));
        assert!(!with(&[
            ("sec-fetch-site", "cross-site"),
            ("origin", "https://portal.example")
        ]));
        assert!(!with(&[
            ("sec-fetch-site", "same-origin"),
            ("sec-fetch-site", "cross-site")
        ]));
        assert!(!with(&[("origin", "https://portal.example.evil.example")]));
        assert!(!with(&[("origin", "null")]));
    }
}
