# Sign-in, roles and sessions (proxy mode)

In a deployment castle runs in **proxy mode** (`settings.auth_mode: proxy`). It
performs no login itself: **oauth2-proxy** does the OIDC flow with **Keycloak**
and forwards the user's identity to castle as request headers. Castle creates
the user on first sight and reads the role (`manager`/`staff`/`client`) from
their Keycloak groups on every request.

```
   browser ──▶ Caddy ──▶ oauth2-proxy ──(OIDC)──▶ Keycloak
                             │  (sets X-Forwarded-User / -Email / -Groups /
                             │   -Preferred-Username, and the secret it shares
                             │   with its castle)
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
`BASE_DOMAIN=<your domain> ./keycloak-realm.sh render <codename>` prints
exactly what it will create. In short:

- TLS required for everything; no self-registration; no self-service password
  reset; email must be verified.
- Temporary lockout after repeated failed sign-ins.
- Access tokens live 5 minutes; a session ends after 30 minutes without use and
  after 10 hours in any case.
- Sign-in and admin events are recorded.
- One confidential client, `castle`: browser code flow only, with a PKCE
  challenge (S256) required, one exact redirect URI, and a mapper that puts
  the user's groups into the token.
- Three groups: `castle-managers`, `castle-staff`, `castle-clients`.

## People and roles

| Group in the realm | Role in the portal |
|--------------------|--------------------|
| `castle-managers` | manager: sees every project of this codename, opens projects, puts people on them |
| `castle-staff` | staff: works on the projects a manager put them on |
| `castle-clients` | client: reads the published findings of their projects and comments |

Someone in none of the three has no role: Keycloak signs them in, and castle
refuses them without making an account. Roles are Keycloak's to give. Castle
reads them on every request and has no way to edit them. A group counts by its
exact name: castle reads the comma-separated list the proxy sends, and a name
that is only nearly one of the three gives nothing.

**Adding a person.** In Keycloak's admin console (reached through an SSH
tunnel: [`compose/docs/admin-rotation.md`](compose/docs/admin-rotation.md)),
in the realm `castle-<codename>`: *Users → Add user* with their email and *Email verified*
on; *Credentials → Set password*, temporary, so they choose their own at first
sign-in; *Groups → Join* one of the three. To make them enrol an authenticator
app at first sign-in, add *Configure OTP* to their required actions.

**Putting them on a project.** A manager opens the project in the portal and
onboards the person by email, as staff or as client. This works before their
first sign-in: castle keeps an `invited` placeholder that becomes the real
account when the person Keycloak vouches for with that address arrives. A
manager can later change that capacity or remove the person from the project.
What someone may do on a project takes both: the role from Keycloak and the
capacity from the manager.

**Taking access away.**

| Step | Takes effect |
|------|--------------|
| disable or delete the user in Keycloak, or remove them from a group | within four minutes, at the session's next refresh |
| `podman exec castle-<codename> castle-cli task --environment production user:status email:<email> status:disabled` | at the next request |
| remove them from a project in the portal | at the next request, for that project |
| `podman exec castle-<codename> castle-cli task --environment production user:anonymize email:<email>` | at the next request: the account is disabled, off every project, and carries a placeholder in place of the name and address |

Someone who leaves for good is anonymized, not deleted, so the findings and
comments they wrote stay in place. The account stays bound to them: should
Keycloak still know the person, they are that disabled account and not a new
one. Delete them in Keycloak as well. An account that nobody ever signed in to
is bound to nobody, so after it is anonymized its former address would start a
new account.

**An account is a person's, not an address's.** Castle binds an account to the
id Keycloak has for the person (`sub`, which the proxy sends as
`X-Forwarded-User`) the first time they arrive. From then on:

- a changed address in Keycloak is the same account, and the address follows
  when no other account has it;
- someone else who is later given that address in Keycloak does not get the
  account: castle refuses them, and nothing changes;
- disabling or anonymizing the account holds whatever address the person comes
  with.

An address finds an account only once, while the account is bound to nobody:
an invitation, or an account that has no id yet. When a person's Keycloak
account was deleted and made anew, they have a new id, and castle refuses them
until their account is let go to whoever next arrives with its address:

```bash
podman exec castle-<codename> castle-cli task --environment production user:unbind email:<email>
```

After moving a tenant to another realm, where everybody has a new id, it is
`user:unbind all:yes`, which `castlectl import` runs by itself.

Managers see everyone with an account, with role, status and projects, under
"People" in the portal; `./castlectl users <codename>` prints what the realm
holds.

## Sessions

- Signing in, the proxy sends the browser to the realm with a PKCE challenge
  and a nonce (`--code-challenge-method=S256`,
  `--insecure-oidc-skip-nonce=false`). Keycloak redeems the code only for the
  verifier the proxy kept for that browser, and the proxy accepts the tokens
  only with that nonce, so a code that reaches another browser is of no use
  there. The realm's client requires the challenge, so Keycloak starts no
  sign-in for it without one.
- The session is an encrypted cookie for that one codename. It is refreshed
  with Keycloak every four minutes (`--cookie-refresh=4m`, shorter than the
  access token's five) and ends after ten hours (`--cookie-expire=10h`).
- So the realm's limits apply to the portal: thirty minutes without use ends
  the session, and so does anything an administrator does to the user or
  their session in Keycloak.
- Sign-out (`/oauth2/sign_out`) ends the Keycloak session as well
  (`--backend-logout-url`). The edge answers that address to a browser that
  goes there and to no request a page makes for an image, a font or a
  stylesheet (`Sec-Fetch-Dest`), so nothing written in a finding or a comment
  signs its readers out.
- The cookie is `SameSite=Lax`, and castle refuses a request that changes
  something unless the browser marks it as coming from the portal's own pages
  (`Sec-Fetch-Site: same-origin`, or the portal's own `Origin` from a browser
  that does not send that header). A link from elsewhere opens the portal; a
  form on another site changes nothing in it.

## Settings

`config/production.yaml`, the `settings.proxy` block. The defaults match the
Compose templates:

```yaml
settings:
  auth_mode: proxy
  proxy:
    subject_header: x-forwarded-user              # Keycloak's id for the person
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
four header names at those. Each is read when it is sent once. The groups
header is a comma-separated list of the groups' names, without paths, which is
what Keycloak's group mapper sends with *Full group path* off.

## Running it somewhere else

[`compose/templates/tenant.compose.yml`](compose/templates/tenant.compose.yml)
is the reference for the proxy's flags, and
[`compose/templates/site.caddy`](compose/templates/site.caddy) for what the edge
must do. Whatever replaces them has to keep the three points above, the
challenge and the nonce at sign-in, a `--cookie-refresh` shorter than the
access token's lifetime, and `--backend-logout-url`, so that signing out of
the portal ends the Keycloak session as well.

## Local development

Development keeps the built-in JWT login (`config/development.yaml` →
`settings.auth_mode: jwt`), so no Keycloak is needed: run `cargo run -- start`
and `npm --prefix frontend run dev`, and sign in with a seeded account.
