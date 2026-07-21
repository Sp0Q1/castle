# Deploying Castle behind oauth2-proxy + Keycloak (SSO)

In production Castle runs in **proxy auth mode** (`settings.auth_mode: proxy`).
It performs no login itself: **oauth2-proxy** does the full OIDC flow with
**Keycloak**, then forwards the user's identity to Castle as request headers.
Castle trusts those headers, provisions/updates the user on the fly, and derives
the platform role (`manager`/`staff`/`client`) from the user's Keycloak groups on
every request.

```
   browser ──▶ oauth2-proxy ──(OIDC)──▶ Keycloak
                   │  (adds X-Forwarded-Email / -Groups / -Preferred-Username)
                   ▼
                 Castle  (private network — NOT publicly reachable)
```

## ⚠️ The one rule that makes this safe

In proxy mode the identity headers are **trusted unconditionally**. If Castle's
port is reachable without going through oauth2-proxy, anyone can send
`X-Forwarded-Email: manager@you.com` and become a manager. Therefore:

1. **Bind Castle to a private interface / internal network only** — never expose
   `:5150` publicly. In the compose below it is only on the internal network.
2. Enforce it at the infra layer too (k8s NetworkPolicy, security group,
   firewall) so only oauth2-proxy can dial Castle.
3. Optionally set `CASTLE_PROXY_SECRET` + an ingress that injects
   `X-Castle-Proxy-Secret` as defense-in-depth (Castle rejects requests without
   it). This complements — does not replace — network isolation.

## Keycloak setup

1. Realm `castle` (or reuse an existing one).
2. Client `castle` — type **confidential** (client authentication on), standard
   flow enabled, redirect URI `https://castle.example.com/oauth2/callback`, and a
   **valid post-logout redirect URI** of `https://castle.example.com/*` (or the
   app root). Sign-out sends `post_logout_redirect_uri=<app root>`; Keycloak
   rejects it — breaking logout with a "We are sorry…" error — unless it's
   registered here. Also ensure oauth2-proxy is started with
   `--whitelist-domain=<keycloak host[:port]>` so it will redirect to the IdP's
   end-session endpoint on sign-out (include the port if non-standard).
   Note: without an `id_token_hint`, Keycloak shows a "Do you want to log out?"
   confirmation page before completing — expected; seamless logout needs the
   hint, which oauth2-proxy's `sign_out` does not add.
3. Groups: create `castle-managers`, `castle-staff`, `castle-clients` and assign
   users. (Unmapped users get least privilege = `client`.)
4. Add a **Group Membership** mapper to the `castle` client (or a client scope it
   uses): name `groups`, token claim name `groups`, "Full group path" OFF, add
   to userinfo ON. This makes Keycloak emit a `groups` claim that oauth2-proxy
   forwards.

## oauth2-proxy + Castle (docker-compose sketch)

```yaml
services:
  oauth2-proxy:
    image: quay.io/oauth2-proxy/oauth2-proxy:latest
    command:
      - --provider=keycloak-oidc
      - --oidc-issuer-url=https://keycloak.example.com/realms/castle
      - --client-id=castle
      - --client-secret=${OIDC_CLIENT_SECRET}
      - --redirect-url=https://castle.example.com/oauth2/callback
      - --cookie-secret=${COOKIE_SECRET}          # 32 random bytes, base64
      - --email-domain=*                           # or restrict to your domain
      - --scope=openid email profile               # do NOT add "groups" here unless
                                                   # you created a `groups` *client scope*
                                                   # in Keycloak — otherwise Keycloak
                                                   # returns invalid_scope. The groups
                                                   # claim comes from the mapper below,
                                                   # which needs no extra scope.
      - --upstream=http://castle:5150              # Castle, on the internal net
      - --pass-user-headers=true                   # -> X-Forwarded-Email/-User/-Groups
      - --set-xauthrequest=true                    # also emits X-Auth-Request-*
      - --http-address=0.0.0.0:4180
      - --reverse-proxy=true
      - --trusted-proxy-ip=<your LB CIDR>          # REQUIRED if behind an LB
    ports: ["4180:4180"]                           # the only public entrypoint
    networks: [edge, internal]

  castle:
    image: castle:latest                           # built from this repo (see below)
    environment:
      LOCO_ENV: production
      DATABASE_URL: postgres://castle:...@db/castle # or sqlite for a single node
      # Base64! loco base64-decodes it. Unused for auth in proxy mode but still parsed:
      JWT_SECRET: ${JWT_SECRET}
      CASTLE_PROXY_SECRET: ${CASTLE_PROXY_SECRET}   # optional defense-in-depth
    networks: [internal]                            # NO published ports

networks:
  edge:
  internal:
    internal: true
```

> Castle has no Dockerfile yet — generate one with `cargo loco generate deployment`
> (choose Docker), which produces a multi-stage build. Run
> `npm --prefix frontend run build` first so `frontend/dist` is baked in and
> served by Castle in production.

If your oauth2-proxy emits `X-Auth-Request-*` instead of `X-Forwarded-*`, just
point Castle at those names in `config/production.yaml`:

```yaml
settings:
  auth_mode: proxy
  proxy:
    email_header: x-auth-request-email
    name_header: x-auth-request-preferred-username
    groups_header: x-auth-request-groups
    manager_group: castle-managers
    staff_group: castle-staff
    client_group: castle-clients
    # shared_secret_header / shared_secret if an ingress injects one
```

## User lifecycle in proxy mode

- **No self-registration** — the `/api/auth/register` and `/login` endpoints are
  not even mounted in proxy mode. Accounts come from SSO + manager onboarding.
- **First sign-in** provisions a Castle user from the proxy identity (keyed by
  email), with the role derived from Keycloak groups.
- **Onboarding someone who hasn't signed in yet** works: a manager onboards them
  by email, which creates an `invited` placeholder user + their project
  membership. On that person's first SSO login the placeholder is matched by
  email and flipped to `active`, keeping the pre-staged membership.
- **Roles are Keycloak-authoritative**: Castle re-derives and caches the role
  from groups on every request and exposes no way to edit it in-app.

## Local development

Dev keeps the built-in JWT login (`config/development.yaml` →
`settings.auth_mode: jwt`) so you don't need Keycloak locally: run
`cargo run -- start` + `npm --prefix frontend run dev` and sign in normally.
