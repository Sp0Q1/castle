#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# keycloak-realm.sh — create, delete or render a tenant's Keycloak realm.
#
# Castle uses realm-per-tenant: a realm is an isolation boundary in Keycloak, so
# one client's users, sessions, password policy and login events cannot reach
# another's. That makes realm creation part of provisioning a tenant, not a
# manual console step — a hand-made realm is where a forgotten `sslRequired` or
# an over-broad redirect URI gets in.
#
#   ./keycloak-realm.sh render  <codename>            # print the realm JSON
#   ./keycloak-realm.sh create  <codename>            # create it (idempotent)
#   ./keycloak-realm.sh delete  <codename>
#
# Env:
#   BASE_DOMAIN     hostname suffix for the tenant       (required for create/render)
#   KC_URL          Keycloak base URL, e.g. https://sso.example.com
#   KC_ADMIN_USER   admin username           (create/delete only)
#   KC_ADMIN_PASS   admin password           (create/delete only)
#   KC_REALM_PREFIX realm name prefix                    (default: castle-)
#   EXTERNAL_PORT   port in browser-facing URLs          (default: none = 443)
#   CLIENT_SECRET   the confidential client's secret; generated if unset. The
#                   SAME value must reach the tenant as OAUTH2_PROXY_CLIENT_SECRET
#                   (castlectl handles this; it is never printed by `create`).
#
# `render` writes JSON to stdout and never contacts Keycloak, so the exact
# object that will be created can be reviewed, diffed and tested offline.
# ---------------------------------------------------------------------------
set -euo pipefail

# TLS verification is on by default and must stay that way: this script sends
# admin credentials and a client secret, so an unverified connection would hand
# both to anyone able to intercept it. KC_INSECURE=1 disables verification for
# local labs only, where the ingress serves a self-signed certificate — it is
# deliberately opt-in and noisy rather than a silent `-k` in the curl calls.
CURL_TLS=()
if [[ "${KC_INSECURE:-0}" == "1" ]]; then
  CURL_TLS=(-k)
  echo "keycloak-realm.sh: WARNING - TLS verification disabled (KC_INSECURE=1)" >&2
fi

cmd="${1:-}"; codename="${2:-}"
[[ -n "$cmd" && -n "$codename" ]] || {
  echo "usage: keycloak-realm.sh {render|create|delete} <codename>" >&2; exit 1; }
[[ "$codename" =~ ^[a-z][a-z0-9-]{1,30}$ ]] || {
  echo "keycloak-realm.sh: invalid codename '$codename'" >&2; exit 1; }

KC_REALM_PREFIX="${KC_REALM_PREFIX:-castle-}"
realm="${KC_REALM_PREFIX}${codename}"

render() {
  local base="${BASE_DOMAIN:?BASE_DOMAIN required}"
  local port="${EXTERNAL_PORT:-}"
  local host="${codename}.${base}${port:+:$port}"
  local secret="${CLIENT_SECRET:-$(openssl rand -base64 32 | tr -d '\n')}"

  # Hardening notes, since these are the settings a console-built realm gets
  # wrong:
  #   sslRequired all       — no plaintext login, ever, including on private nets.
  #   registrationAllowed   — false: castle is invite-only; a self-service signup
  #                           page on a client's realm would be an open door.
  #   resetPasswordAllowed  — false: recovery goes through the reporting team, so
  #                           an attacker who knows a client address cannot mail
  #                           themselves into the flow.
  #   verifyEmail           — true: an invite must be proven to reach its owner.
  #   bruteForceProtected   — on, temporary lockout. Permanent lockout would let
  #                           anyone lock a client out of their own report by
  #                           guessing at their address.
  #   directAccessGrants /  — off: the browser code flow is the only way in.
  #     implicitFlow          Direct grants would hand password auth back to any
  #                           client that asks; implicit leaks tokens in URLs.
  #   redirectUris          — exactly one callback, no wildcard. webOrigins is
  #                           the single origin rather than "+" so CORS cannot be
  #                           widened by a stray redirect entry.
  #   post.logout.redirect  — MUST cover the app root or RP-initiated logout dies
  #                           on Keycloak's "We are sorry..." page. This bit us in
  #                           the lab; see deploy/oauth2-proxy-keycloak.md.
  cat <<JSON
{
  "realm": "${realm}",
  "enabled": true,
  "displayName": "${codename}",
  "sslRequired": "all",
  "registrationAllowed": false,
  "resetPasswordAllowed": false,
  "rememberMe": false,
  "verifyEmail": true,
  "loginWithEmailAllowed": true,
  "duplicateEmailsAllowed": false,
  "bruteForceProtected": true,
  "permanentLockout": false,
  "failureFactor": 8,
  "waitIncrementSeconds": 60,
  "maxFailureWaitSeconds": 900,
  "quickLoginCheckMilliSeconds": 1000,
  "minimumQuickLoginWaitSeconds": 60,
  "accessTokenLifespan": 300,
  "ssoSessionIdleTimeout": 1800,
  "ssoSessionMaxLifespan": 36000,
  "eventsEnabled": true,
  "eventsExpiration": 604800,
  "adminEventsEnabled": true,
  "adminEventsDetailsEnabled": true,
  "groups": [
    {"name": "castle-managers"},
    {"name": "castle-staff"},
    {"name": "castle-clients"}
  ],
  "clients": [
    {
      "clientId": "castle",
      "enabled": true,
      "publicClient": false,
      "secret": "${secret}",
      "standardFlowEnabled": true,
      "directAccessGrantsEnabled": false,
      "implicitFlowEnabled": false,
      "serviceAccountsEnabled": false,
      "fullScopeAllowed": false,
      "redirectUris": ["https://${host}/oauth2/callback"],
      "webOrigins": ["https://${host}"],
      "attributes": {
        "post.logout.redirect.uris": "https://${host}/*",
        "backchannel.logout.session.required": "true"
      },
      "protocolMappers": [
        {
          "name": "groups",
          "protocol": "openid-connect",
          "protocolMapper": "oidc-group-membership-mapper",
          "config": {
            "full.path": "false",
            "claim.name": "groups",
            "access.token.claim": "true",
            "id.token.claim": "true",
            "userinfo.token.claim": "true"
          }
        }
      ]
    }
  ],
  "users": []
}
JSON
}

# Admin API token. Uses the password grant against the master realm's built-in
# admin-cli client, which is how Keycloak's own kcadm authenticates.
kc_token() {
  local url="${KC_URL:?KC_URL required}"
  curl -sSf "${CURL_TLS[@]}" --data-urlencode "username=${KC_ADMIN_USER:?KC_ADMIN_USER required}" \
       --data-urlencode "password=${KC_ADMIN_PASS:?KC_ADMIN_PASS required}" \
       -d "grant_type=password" -d "client_id=admin-cli" \
       "${url}/realms/master/protocol/openid-connect/token" \
    | sed -n 's/.*"access_token":"\([^"]*\)".*/\1/p'
}

realm_exists() {
  local url="$1" token="$2"
  curl -sS "${CURL_TLS[@]}" -o /dev/null -w '%{http_code}' \
    -H "Authorization: Bearer ${token}" "${url}/admin/realms/${realm}" | grep -q '^200$'
}

create() {
  local url="${KC_URL:?KC_URL required}" token; token="$(kc_token)"
  [[ -n "$token" ]] || { echo "keycloak-realm.sh: could not obtain admin token" >&2; exit 1; }

  if realm_exists "$url" "$token"; then
    # Idempotent on purpose: provisioning is re-run after partial failures, and
    # re-creating a live realm would invalidate every existing session.
    echo "realm ${realm} already exists — leaving it untouched" >&2
    return 0
  fi

  # The realm JSON carries the client secret, so it goes over the pipe and never
  # onto disk or into the process list.
  render | curl -sSf "${CURL_TLS[@]}" -X POST "${url}/admin/realms" \
      -H "Authorization: Bearer ${token}" \
      -H "Content-Type: application/json" \
      --data-binary @- >/dev/null
  echo "created realm ${realm}" >&2
}

delete() {
  local url="${KC_URL:?KC_URL required}" token; token="$(kc_token)"
  curl -sSf "${CURL_TLS[@]}" -X DELETE "${url}/admin/realms/${realm}" \
    -H "Authorization: Bearer ${token}" >/dev/null
  echo "deleted realm ${realm}" >&2
}

case "$cmd" in
  render) render ;;
  create) create ;;
  delete) delete ;;
  *) echo "usage: keycloak-realm.sh {render|create|delete} <codename>" >&2; exit 1 ;;
esac
