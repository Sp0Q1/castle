use crate::rate_limit;
use crate::security::CurrentUser;
use crate::{
    mailers::auth::AuthMailer,
    models::{
        _entities::users,
        users::{LoginParams, RegisterParams},
    },
    views::auth::{CurrentResponse, LoginResponse},
};
use axum::http::HeaderMap;
use loco_rs::prelude::*;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

pub static EMAIL_DOMAIN_RE: OnceLock<Regex> = OnceLock::new();

fn get_allow_email_domain_re() -> &'static Regex {
    EMAIL_DOMAIN_RE.get_or_init(|| {
        Regex::new(r"@example\.com$|@gmail\.com$").expect("Failed to compile regex")
    })
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ForgotParams {
    pub email: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ResetParams {
    pub token: String,
    pub password: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MagicLinkParams {
    pub email: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ResendVerificationParams {
    pub email: String,
}

/// Register function creates a new user with the given parameters and sends a
/// welcome email to the user
#[debug_handler]
async fn register(
    State(ctx): State<AppContext>,
    Json(params): Json<RegisterParams>,
) -> Result<Response> {
    // Registration and the two recovery flows below all answer 200 regardless
    // of whether the address exists, so they cannot be used to enumerate
    // accounts one request at a time — but they can be used to mail-bomb a
    // known address, and they each cost a database lookup.
    rate_limit::check_signup(&params.email)?;

    let res = users::Model::create_with_password(&ctx.db, &params).await;

    let user = match res {
        Ok(user) => user,
        Err(err) => {
            tracing::info!(
                message = err.to_string(),
                user_email = &params.email,
                "could not register user",
            );
            return format::json(());
        }
    };

    let user = user
        .into_active_model()
        .set_email_verification_sent(&ctx.db)
        .await?;

    AuthMailer::send_welcome(&ctx, &user).await?;

    format::json(())
}

/// Verify register user. if the user not verified his email, he can't login to
/// the system.
#[debug_handler]
async fn verify(State(ctx): State<AppContext>, Path(token): Path<String>) -> Result<Response> {
    let Ok(user) = users::Model::find_by_verification_token(&ctx.db, &token).await else {
        return unauthorized("invalid token");
    };

    if user.email_verified_at.is_some() {
        tracing::info!(pid = user.pid.to_string(), "user already verified");
    } else {
        let active_model = user.into_active_model();
        let user = active_model.verified(&ctx.db).await?;
        tracing::info!(pid = user.pid.to_string(), "user verified");
    }

    format::json(())
}

/// In case the user forgot his password  this endpoints generate a forgot token
/// and send email to the user. In case the email not found in our DB, we are
/// returning a valid request for for security reasons (not exposing users DB
/// list).
#[debug_handler]
async fn forgot(
    State(ctx): State<AppContext>,
    Json(params): Json<ForgotParams>,
) -> Result<Response> {
    rate_limit::check_signup(&params.email)?;

    let Ok(user) = users::Model::find_by_email(&ctx.db, &params.email).await else {
        // we don't want to expose our users email. if the email is invalid we still
        // returning success to the caller
        return format::json(());
    };

    let user = user
        .into_active_model()
        .set_forgot_password_sent(&ctx.db)
        .await?;

    AuthMailer::forgot_password(&ctx, &user).await?;

    format::json(())
}

/// reset user password by the given parameters
#[debug_handler]
async fn reset(State(ctx): State<AppContext>, Json(params): Json<ResetParams>) -> Result<Response> {
    let Ok(user) = users::Model::find_by_reset_token(&ctx.db, &params.token).await else {
        // we don't want to expose our users email. if the email is invalid we still
        // returning success to the caller
        tracing::info!("reset token not found");

        return format::json(());
    };
    user.into_active_model()
        .reset_password(&ctx.db, &params.password)
        .await?;

    format::json(())
}

/// Creates a user login and returns a token
#[debug_handler]
async fn login(State(ctx): State<AppContext>, Json(params): Json<LoginParams>) -> Result<Response> {
    // Checked before the lookup so that throttling costs an attacker a request
    // regardless of whether the account exists — and so a locked-out account
    // never reaches password verification.
    rate_limit::check_login(&params.email)?;

    let Ok(user) = users::Model::find_by_email(&ctx.db, &params.email).await else {
        tracing::debug!(
            email = params.email,
            "login attempt with non-existent email"
        );
        return unauthorized("Invalid credentials!");
    };

    let valid = user.verify_password(&params.password);

    if !valid {
        return unauthorized("unauthorized!");
    }

    let jwt_secret = ctx.config.get_jwt_config()?;

    let token = user
        .generate_jwt(&jwt_secret.secret, jwt_secret.expiration)
        .or_else(|_| unauthorized("unauthorized!"))?;

    // Authentication succeeded, so the earlier failures were almost certainly
    // typos, not an attack -- don't leave this user one mistake from a lockout.
    rate_limit::clear_login(&params.email);

    format::json(LoginResponse::new(&user, &token))
}

#[debug_handler]
async fn current(
    CurrentUser(user): CurrentUser,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    // Surface the sign-out URL only in proxy mode (JWT mode logs out locally).
    let settings = crate::security::Settings::from_ctx(&ctx);
    let logout_url = match settings.auth_mode {
        crate::security::AuthMode::Proxy => settings.proxy.logout_url,
        crate::security::AuthMode::Jwt => None,
    };
    format::json(CurrentResponse::new(&user, logout_url))
}

#[derive(Serialize)]
struct ModeResponse {
    mode: &'static str,
}

/// Public (unauthenticated) endpoint reporting the auth mode so the SPA knows
/// whether to render its own login form (`jwt`) or hand off to the proxy/IdP
/// (`proxy`) on an unauthenticated request. Exposes nothing sensitive. In proxy
/// mode oauth2-proxy must be told to skip auth for this path.
#[debug_handler]
async fn mode(State(ctx): State<AppContext>) -> Result<Response> {
    let mode = match crate::security::Settings::from_ctx(&ctx).auth_mode {
        crate::security::AuthMode::Proxy => "proxy",
        crate::security::AuthMode::Jwt => "jwt",
    };
    format::json(ModeResponse { mode })
}

/// Magic link authentication provides a secure and passwordless way to log in to the application.
///
/// # Flow
/// 1. **Request a Magic Link**:
///    A registered user sends a POST request to `/magic-link` with their email.
///    If the email exists, a short-lived, one-time-use token is generated and sent to the user's email.
///    For security and to avoid exposing whether an email exists, the response always returns 200, even if the email is invalid.
///
/// 2. **Click the Magic Link**:
///    The user clicks the link (/magic-link/{token}), which validates the token and its expiration.
///    If valid, the server generates a JWT and responds with a [`LoginResponse`].
///    If invalid or expired, an unauthorized response is returned.
///
/// This flow enhances security by avoiding traditional passwords and providing a seamless login experience.
async fn magic_link(
    State(ctx): State<AppContext>,
    Json(params): Json<MagicLinkParams>,
) -> Result<Response> {
    rate_limit::check_signup(&params.email)?;

    let email_regex = get_allow_email_domain_re();
    if !email_regex.is_match(&params.email) {
        tracing::debug!(
            email = params.email,
            "The provided email is invalid or does not match the allowed domains"
        );
        return bad_request("invalid request");
    }

    let Ok(user) = users::Model::find_by_email(&ctx.db, &params.email).await else {
        // we don't want to expose our users email. if the email is invalid we still
        // returning success to the caller
        tracing::debug!(email = params.email, "user not found by email");
        return format::empty_json();
    };

    let user = user.into_active_model().create_magic_link(&ctx.db).await?;
    AuthMailer::send_magic_link(&ctx, &user).await?;

    format::empty_json()
}

/// Verifies a magic link token and authenticates the user.
async fn magic_link_verify(
    Path(token): Path<String>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let Ok(user) = users::Model::find_by_magic_token(&ctx.db, &token).await else {
        // we don't want to expose our users email. if the email is invalid we still
        // returning success to the caller
        return unauthorized("unauthorized!");
    };

    let user = user.into_active_model().clear_magic_link(&ctx.db).await?;

    let jwt_secret = ctx.config.get_jwt_config()?;

    let token = user
        .generate_jwt(&jwt_secret.secret, jwt_secret.expiration)
        .or_else(|_| unauthorized("unauthorized!"))?;

    format::json(LoginResponse::new(&user, &token))
}

#[debug_handler]
async fn resend_verification_email(
    State(ctx): State<AppContext>,
    Json(params): Json<ResendVerificationParams>,
) -> Result<Response> {
    rate_limit::check_signup(&params.email)?;

    let Ok(user) = users::Model::find_by_email(&ctx.db, &params.email).await else {
        tracing::info!(
            email = params.email,
            "User not found for resend verification"
        );
        return format::json(());
    };

    if user.email_verified_at.is_some() {
        tracing::info!(
            pid = user.pid.to_string(),
            "User already verified, skipping resend"
        );
        return format::json(());
    }

    let user = user
        .into_active_model()
        .set_email_verification_sent(&ctx.db)
        .await?;

    AuthMailer::send_welcome(&ctx, &user).await?;
    tracing::info!(pid = user.pid.to_string(), "Verification email re-sent");

    format::json(())
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/auth")
        .add("/register", post(register))
        .add("/verify/{token}", get(verify))
        .add("/login", post(login))
        .add("/forgot", post(forgot))
        .add("/reset", post(reset))
        .add("/current", get(current))
        .add("/magic-link", post(magic_link))
        .add("/magic-link/{token}", get(magic_link_verify))
        .add("/resend-verification-mail", post(resend_verification_email))
        .add("/mode", get(mode))
}

/// Routes for `proxy` auth mode: only the current-user endpoint. The credential
/// endpoints (register/login/forgot/reset/magic-link) are intentionally omitted
/// because oauth2-proxy handles authentication and the app manages no passwords.
pub fn proxy_routes() -> Routes {
    Routes::new()
        .prefix("/api/auth")
        .add("/current", get(current))
        .add("/mode", get(mode))
}

// ---------------------------------------------------------------------------
// Honeypot mode. Decoy instances present the SAME auth surface as `routes()`,
// but every handler captures the submitted credentials + attacker metadata to
// the `castle::honeypot` log target (shipped to the deception pipeline) and
// returns a believable response — no DB writes, no real accounts.
// ---------------------------------------------------------------------------

fn honeypot_capture(event: &str, headers: &HeaderMap, submitted: &str) {
    let h = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).unwrap_or("-");
    tracing::warn!(
        target: "castle::honeypot",
        signal = "deception",
        event = event,
        submitted = submitted,
        source_ip = h("x-forwarded-for"),
        real_ip = h("x-real-ip"),
        user_agent = h("user-agent"),
        referer = h("referer"),
        "honeypot credential capture"
    );
}

// The decoys apply exactly the same limits as a real instance, at the same
// thresholds, returning the same 429 body. A decoy that let an attacker guess
// forever while production locked out after eight tries would be trivially
// distinguishable by hammering it — which would undo the whole point of the
// swarm. Capture happens *before* the limit check, so throttled attempts are
// still recorded: the attacker sees an ordinary lockout, we keep the intel.
#[debug_handler]
async fn hp_register(headers: HeaderMap, Json(params): Json<RegisterParams>) -> Result<Response> {
    honeypot_capture(
        "register",
        &headers,
        &format!(
            "email={} password={} name={}",
            params.email, params.password, params.name
        ),
    );
    rate_limit::check_signup(&params.email)?;
    // Believable success — but no account is created.
    format::json(())
}

#[debug_handler]
async fn hp_login(headers: HeaderMap, Json(params): Json<LoginParams>) -> Result<Response> {
    honeypot_capture(
        "login",
        &headers,
        &format!("email={} password={}", params.email, params.password),
    );
    rate_limit::check_login(&params.email)?;
    unauthorized("Invalid credentials!")
}

#[debug_handler]
async fn hp_forgot(headers: HeaderMap, Json(params): Json<ForgotParams>) -> Result<Response> {
    honeypot_capture("forgot", &headers, &format!("email={}", params.email));
    rate_limit::check_signup(&params.email)?;
    format::json(())
}

#[debug_handler]
async fn hp_reset(headers: HeaderMap, Json(params): Json<ResetParams>) -> Result<Response> {
    honeypot_capture(
        "reset",
        &headers,
        &format!("token={} password={}", params.token, params.password),
    );
    format::json(())
}

#[debug_handler]
async fn hp_magic_link(
    headers: HeaderMap,
    Json(params): Json<MagicLinkParams>,
) -> Result<Response> {
    honeypot_capture("magic_link", &headers, &format!("email={}", params.email));
    format::empty_json()
}

#[debug_handler]
async fn hp_token_path(headers: HeaderMap, Path(token): Path<String>) -> Result<Response> {
    honeypot_capture("token_path", &headers, &format!("token={token}"));
    unauthorized("unauthorized!")
}

#[debug_handler]
async fn hp_resend(
    headers: HeaderMap,
    Json(params): Json<ResendVerificationParams>,
) -> Result<Response> {
    honeypot_capture(
        "resend_verification",
        &headers,
        &format!("email={}", params.email),
    );
    format::json(())
}

#[debug_handler]
async fn hp_current(headers: HeaderMap) -> Result<Response> {
    honeypot_capture("current", &headers, "-");
    unauthorized("unauthorized!")
}

/// Routes for honeypot/decoy instances — identical paths to `routes()`, but each
/// handler captures instead of authenticating.
pub fn honeypot_routes() -> Routes {
    Routes::new()
        .prefix("/api/auth")
        .add("/register", post(hp_register))
        .add("/verify/{token}", get(hp_token_path))
        .add("/login", post(hp_login))
        .add("/forgot", post(hp_forgot))
        .add("/reset", post(hp_reset))
        .add("/current", get(hp_current))
        .add("/magic-link", post(hp_magic_link))
        .add("/magic-link/{token}", get(hp_token_path))
        .add("/resend-verification-mail", post(hp_resend))
        .add("/mode", get(mode))
}
