# Sign-in, roles and sessions (proxy mode)

In a deployment castle runs in **proxy mode** (`settings.auth_mode: proxy`). It
performs no login itself: **oauth2-proxy** does the OIDC flow with **Keycloak**
and forwards the user's identity to castle as request headers. Castle creates
the user on first sight and reads the role (`manager`/`staff`/`client`) from
their Keycloak groups on every request.

```
   browser ──▶ Caddy ──▶ oauth2-proxy ──(OIDC)──▶ Keycloak
                             │  (sets X-Forwarded-Email / -Groups / -Preferred-Username,
                             │   and the secret it shares with its castle)
                             ▼
                           castle  (its tenant's internal network, no published port)
```

## What castle believes, and from whom

Castle takes who is asking from request headers, so everything rests on who
can set them. Three things keep that to the proxy, and the Compose templates
do all three:

1. **The proxy proves itself to castle.** oauth2-proxy runs with
   `--pass-basic-auth=true` and a password (`OAUTH2_PROXY_BASIC_AUTH_PASSWORD`),
   so everything it forwards carries that password in a Basic `Authorization`
   header, in place of whatever the browser sent under that name. Castle gets
   the same value as `CASTLE_PROXY_SECRET`. It reads the identity headers only
   from a request that carries it, and does not start in proxy mode without
   one. Without it, every address that needs to know who is asking answers
   401; what needs no identity is answered as before: the auth mode, the logo,
   the pages' own files and the three health addresses. `castlectl` generates
   one secret per codename.
2. **Castle's network is closed.** No published port, and a network marked
   `internal` that only its proxy and its database join. podman routes between
   ordinary bridge networks, so the mark is what keeps the rest of the host
   out.
3. **The edge decides what the proxy believes.** oauth2-proxy runs with
   `--reverse-proxy=true` and takes `X-Forwarded-Uri` and `X-Real-IP` from
   whatever is in front of it. The vhost template removes the first and sets
   the second, and no path is exempt from sign-in.

## The realm

`keycloak-realm.sh` creates one realm per codename, and
`./keycloak-realm.sh render <codename>` prints exactly what it will create. In
short:

- TLS required for everything; no self-registration; no self-service password
  reset; email must be verified.
- Temporary lockout after repeated failed sign-ins.
- Access tokens live 5 minutes; a session ends after 30 minutes without use and
  after 10 hours in any case.
- Sign-in and admin events are recorded.
- One confidential client, `castle`: browser code flow only, one exact
  redirect URI, and a mapper that puts the user's groups into the token.
- Three groups: `castle-managers`, `castle-staff`, `castle-clients`.

## People and roles

| Group in the realm | Role in the portal |
|--------------------|--------------------|
| `castle-managers` | manager: sees every project of this codename, opens projects, puts people on them |
| `castle-staff` | staff: works on the projects a manager put them on |
| `castle-clients`, or no group | client: reads the published findings of their projects and comments |

Roles are Keycloak's to give. Castle reads them on every request and has no
way to edit them.

**Adding a person.** In Keycloak's admin console, in the realm
`castle-<codename>`: *Users → Add user* with their email and *Email verified*
on; *Credentials → Set password*, temporary, so they choose their own at first
sign-in; *Groups → Join* one of the three. To make them enrol an authenticator
app at first sign-in, add *Configure OTP* to their required actions.

**Putting them on a project.** A manager opens the project in the portal and
onboards the person by email, as staff or as client. This works before their
first sign-in: castle keeps an `invited` placeholder that becomes the real
account when they arrive. A manager can later change that capacity or remove
the person from the project. What someone may do on a project takes both: the
role from Keycloak and the capacity from the manager.

**Taking access away.**

| Step | Takes effect |
|------|--------------|
| disable or delete the user in Keycloak, or remove them from a group | within four minutes, at the session's next refresh |
| `podman exec castle-<codename> castle-cli task --environment production user:status email:<email> status:disabled` | at the next request |
| remove them from a project in the portal | at the next request, for that project |
| `podman exec castle-<codename> castle-cli task --environment production user:anonymize email:<email>` | at the next request: the account is disabled, off every project, and carries a placeholder in place of the name and address |

Someone who leaves for good is anonymized, not deleted, so the findings and
comments they wrote stay in place. Delete them in Keycloak as well, or their
next sign-in starts a new account.

Managers see everyone with an account, with role, status and projects, under
"People" in the portal; `./castlectl users <codename>` prints what the realm
holds.

## Sessions

- Signing in, the proxy sends the browser to the realm with a PKCE challenge
  and a nonce (`--code-challenge-method=S256`,
  `--insecure-oidc-skip-nonce=false`). Keycloak redeems the code only for the
  verifier the proxy kept for that browser, and the proxy accepts the tokens
  only with that nonce, so a code that reaches another browser is of no use
  there.
- The session is an encrypted cookie for that one codename. It is refreshed
  with Keycloak every four minutes (`--cookie-refresh=4m`, shorter than the
  access token's five) and ends after ten hours (`--cookie-expire=10h`).
- So the realm's limits apply to the portal: thirty minutes without use ends
  the session, and so does anything an administrator does to the user or
  their session in Keycloak.
- Sign-out (`/oauth2/sign_out`) ends the Keycloak session as well
  (`--backend-logout-url`).

## Settings

`config/production.yaml`, the `settings.proxy` block. The defaults match the
Compose templates:

```yaml
settings:
  auth_mode: proxy
  proxy:
    email_header: x-forwarded-email
    name_header: x-forwarded-preferred-username
    groups_header: x-forwarded-groups
    manager_group: castle-managers
    staff_group: castle-staff
    client_group: castle-clients
    secret: …                                     # CASTLE_PROXY_SECRET, required
    logout_url: /oauth2/sign_out?rd=/             # CASTLE_LOGOUT_URL
```

If your proxy sends `X-Auth-Request-*` instead of `X-Forwarded-*`, point the
three header names at those.

## Running it somewhere else

[`compose/templates/tenant.compose.yml`](compose/templates/tenant.compose.yml)
is the reference for the proxy's flags, and
[`compose/templates/site.caddy`](compose/templates/site.caddy) for what the edge
must do. Whatever replaces them has to keep the three points above, a
`--cookie-refresh` shorter than the access token's lifetime, and a Keycloak
client whose post-logout redirect URI covers the portal's root.

## Local development

Development keeps the built-in JWT login (`config/development.yaml` →
`settings.auth_mode: jwt`), so no Keycloak is needed: run `cargo run -- start`
and `npm --prefix frontend run dev`, and sign in with a seeded account.
