#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# keycloak-realm.sh — create, delete or render a tenant's Keycloak realm, and
# list or add the people in it.
#
# Castle uses realm-per-tenant: a realm is an isolation boundary in Keycloak, so
# one client's users, sessions, password policy and login events cannot reach
# another's. That makes realm creation part of provisioning a tenant, not a
# manual console step — a hand-made realm is where a forgotten `sslRequired` or
# an over-broad redirect URI gets in.
#
#   ./keycloak-realm.sh render  <codename>            # print the realm JSON
#   ./keycloak-realm.sh create  <codename>            # create it (idempotent)
#   ./keycloak-realm.sh update  <codename>            # the attributes of an existing realm's client, as `create` sets them
#   ./keycloak-realm.sh delete  <codename>            # with everyone in it (idempotent)
#   ./keycloak-realm.sh people  <codename>            # print who is in it, as JSON
#   ./keycloak-realm.sh add-people <codename> < json  # create them, without credentials
#   ./keycloak-realm.sh await                         # wait until Keycloak answers
#
# Env:
#   BASE_DOMAIN     hostname suffix for the tenant       (required for create, update and render)
#   KC_URL          Keycloak base URL, e.g. https://sso.example.com
#   KC_ADMIN_USER   admin username           (create/delete only)
#   KC_ADMIN_PASS   admin password           (create/delete only)
#   KC_REALM_PREFIX realm name prefix                    (default: castle-)
#   KC_WAIT         how many seconds `await` waits for Keycloak (default: 120)
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

usage() {
  echo "usage: keycloak-realm.sh {render|create|update|delete|people|add-people} <codename>" >&2
  echo "       keycloak-realm.sh await" >&2
  exit 1
}
cmd="${1:-}"; codename="${2:-}"
case "$cmd" in
  # About Keycloak itself, not about a codename's realm.
  await) [[ -z "$codename" ]] || usage ;;
  *)
    [[ -n "$cmd" && -n "$codename" ]] || usage
    [[ "$codename" =~ ^[a-z][a-z0-9-]{1,30}$ ]] || {
      echo "keycloak-realm.sh: invalid codename '$codename'" >&2; exit 1; }
    ;;
esac

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
  #                           on Keycloak's "We are sorry..." page.
  #   pkce.code.challenge   — S256 required: Keycloak starts no sign-in for this
  #     .method               client without a challenge, and redeems a code only
  #                           for the verifier that goes with it. The proxy sends
  #                           one (--code-challenge-method=S256).
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
      "baseUrl": "https://${host}/",
      "alwaysDisplayInConsole": true,
      "redirectUris": ["https://${host}/oauth2/callback"],
      "webOrigins": ["https://${host}"],
      "attributes": {
        "post.logout.redirect.uris": "https://${host}/*",
        "backchannel.logout.session.required": "true",
        "pkce.code.challenge.method": "S256"
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
# admin-cli client, which is how Keycloak's own kcadm authenticates. The
# password reaches curl on standard input: an argument would sit in the process
# list, readable by every user of the host.
kc_token() {
  local url="${KC_URL:?KC_URL required}"
  printf '%s' "${KC_ADMIN_PASS:?KC_ADMIN_PASS required}" \
    | curl -sSf "${CURL_TLS[@]}" --data-urlencode "username=${KC_ADMIN_USER:?KC_ADMIN_USER required}" \
        --data-urlencode "password@-" \
        -d "grant_type=password" -d "client_id=admin-cli" \
        "${url}/realms/master/protocol/openid-connect/token" \
    | jq -er '.access_token'
}

# The token as a header file for curl (`-H @<(bearer "$token")`), for the same
# reason: not an argument.
bearer() { printf 'Authorization: Bearer %s\n' "$1"; }

# Whether the realm exists. Only Keycloak saying so counts: 200 is yes, 404 is
# no, and anything else (it could not be asked, or it refused) stops the script
# rather than pass for either.
realm_exists() {
  local url="$1" token="$2" status
  status="$(curl -sS "${CURL_TLS[@]}" -o /dev/null -w '%{http_code}' \
    -H @<(bearer "$token") "${url}/admin/realms/${realm}")" || status="unreachable"
  case "$status" in
    200) return 0 ;;
    404) return 1 ;;
    *) echo "keycloak-realm.sh: could not tell whether realm ${realm} exists (${status})" >&2; exit 1 ;;
  esac
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
      -H @<(bearer "$token") \
      -H "Content-Type: application/json" \
      --data-binary @- >/dev/null
  echo "created realm ${realm}" >&2
}

delete() {
  local url="${KC_URL:?KC_URL required}" token; token="$(kc_token)"
  [[ -n "$token" ]] || { echo "keycloak-realm.sh: could not obtain admin token" >&2; exit 1; }
  if ! realm_exists "$url" "$token"; then
    echo "realm ${realm} does not exist — nothing to delete" >&2
    return 0
  fi
  curl -sSf "${CURL_TLS[@]}" -X DELETE "${url}/admin/realms/${realm}" \
    -H @<(bearer "$token") >/dev/null
  echo "deleted realm ${realm}" >&2
}

# One authenticated call to the realm's admin API. The admin token lives for a
# minute, so each call fetches its own rather than share one across a long list.
kc() { # <path under the realm> [curl args...]
  local url="${KC_URL:?KC_URL required}" path="$1" token; shift
  token="$(kc_token)"
  [[ -n "$token" ]] || { echo "keycloak-realm.sh: could not obtain admin token" >&2; return 1; }
  curl -sSf "${CURL_TLS[@]}" -H @<(bearer "$token") "$@" "${url}/admin/realms/${realm}${path}"
}

# Who is in the realm and which groups give them a role: a JSON array on stdout.
# No credentials: a password hash or an authenticator's secret stays in Keycloak.
people() {
  local first=0 page user id groups
  while :; do
    page="$(kc "/users?first=${first}&max=100")"
    [[ "$(jq length <<<"$page")" -gt 0 ]] || break
    while read -r user; do
      id="$(jq -r .id <<<"$user")"
      groups="$(kc "/users/${id}/groups" | jq -c '[.[].name]')"
      jq -c --argjson groups "$groups" \
        '{username, email, firstName, lastName, enabled, emailVerified, groups: $groups}' <<<"$user"
    done < <(jq -c '.[]' <<<"$page")
    first=$((first + 100))
  done | jq -s .
}

# Creates the people of a `people` listing (stdin) in this realm, in the groups
# they were in. They get no credentials, so nobody can sign in until an admin
# gives them a password. Someone already there is left as they are.
add_people() {
  local known person username status id group gid
  local -a joined
  known="$(kc "/groups" | jq -c 'map({(.name): .id}) | add // {}')"
  while read -r person; do
    username="$(jq -r .username <<<"$person")"
    status="$(jq -c '{username, email, firstName, lastName, enabled, emailVerified}' <<<"$person" \
      | kc "/users" -o /dev/null -w '%{http_code}' -X POST -H "Content-Type: application/json" --data-binary @- 2>/dev/null || true)"
    case "$status" in
      201) ;;
      409) echo "  ${username}: already there, left as is" >&2; continue ;;
      *)   echo "keycloak-realm.sh: could not add ${username} (HTTP ${status})" >&2; exit 1 ;;
    esac
    id="$(kc "/users" -G --data-urlencode "username=${username}" --data-urlencode "exact=true" | jq -r '.[0].id')"
    joined=()
    while read -r group; do
      gid="$(jq -r --arg g "$group" '.[$g] // empty' <<<"$known")"
      [[ -n "$gid" ]] || { echo "  ${username}: no group '${group}' in ${realm}, skipped" >&2; continue; }
      kc "/users/${id}/groups/${gid}" -o /dev/null -X PUT
      joined+=("$group")
    done < <(jq -r '.groups[]' <<<"$person")
    echo "  ${username}: added (${joined[*]:-no group})" >&2
  done < <(jq -c '.[]')
}

# Brings the `castle` client of an existing realm in line with what `render`
# makes today: its attributes, which is where a setting lives that came after
# the realm did (the PKCE requirement). `create` leaves an existing realm
# alone, so this is how every realm of a pool gets the same client, and a realm
# cannot be told from another by its age. Nothing else of the realm is touched,
# and a client that is already as it should be is not written to.
update() {
  local found id wanted current
  found="$(kc "/clients" -G --data-urlencode "clientId=castle")" \
    || { echo "keycloak-realm.sh: Keycloak did not answer for realm ${realm}" >&2; exit 1; }
  id="$(jq -er '.[0].id' <<<"$found" 2>/dev/null)" \
    || { echo "keycloak-realm.sh: no castle client in realm ${realm}" >&2; exit 1; }
  wanted="$(render | jq -c '.clients[0].attributes')"
  current="$(kc "/clients/${id}")"
  if jq -e --argjson wanted "$wanted" '.attributes as $has | $wanted | to_entries | all($has[.key] == .value)' \
      <<<"$current" >/dev/null; then
    echo "realm ${realm}: the client is as it should be" >&2
    return 0
  fi
  # The secret is left out of what is sent back, which leaves it as it is.
  jq -c --argjson wanted "$wanted" 'del(.secret) | .attributes += $wanted' <<<"$current" \
    | kc "/clients/${id}" -o /dev/null -X PUT -H "Content-Type: application/json" --data-binary @-
  echo "realm ${realm}: the client was brought up to date" >&2
}

# Keycloak takes a minute to answer after it is started. Waits until it does,
# for KC_WAIT seconds at most.
await() {
  local url="${KC_URL:?KC_URL required}" waited=0
  until curl -sSf "${CURL_TLS[@]}" -m 5 -o /dev/null "${url}/realms/master/.well-known/openid-configuration" 2>/dev/null; do
    (( waited < ${KC_WAIT:-120} )) || { echo "keycloak-realm.sh: Keycloak does not answer at ${url}" >&2; exit 1; }
    sleep 3; waited=$((waited + 3))
  done
}

case "$cmd" in
  render) render ;;
  create) create ;;
  update) update ;;
  delete) delete ;;
  people) people ;;
  add-people) add_people ;;
  await) await ;;
  *) usage ;;
esac
