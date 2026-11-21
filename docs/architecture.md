# Architecture and security model

How a deployment is put together, how a request and a sign-in travel through
it, and what the design defends against. Setting one up is in
[`deploy/compose/README.md`](../deploy/compose/README.md).

## One host

Every client gets a codename. A codename is a hostname, a Keycloak realm and,
once it is in use, its own castle and database. Codenames that are not in use
are canaries: they look the same from outside and have nothing behind them.

```mermaid
flowchart TB
  browser([Browser])
  subgraph host["One host"]
    caddy["Caddy: TLS, routing, access log"]
    dns["BIND, optional: authoritative DNS for the pool"]
    subgraph signin["Sign-in"]
      kc["Keycloak: one realm per codename"]
      kcdb[("Keycloak database")]
    end
    subgraph tenant["A tenant: one codename in use"]
      proxy["oauth2-proxy"]
      app["castle"]
      db[("Postgres")]
      files[("uploads")]
    end
    subgraph canary["A canary: every other codename"]
      cproxy["oauth2-proxy"]
      nothing["fixed answer, no backend"]
    end
  end
  browser -->|"codename.domain"| caddy
  browser -->|"sso.domain"| caddy
  caddy --> proxy --> app --> db
  app --> files
  caddy --> cproxy --> nothing
  caddy --> kc --> kcdb
  proxy -.->|"tokens, keys, user info"| kc
  cproxy -.->|"tokens, keys"| kc
```

| Network | Joined by | Carries |
|---------|-----------|---------|
| `castle_edge` | Caddy, Keycloak, every oauth2-proxy | requests coming in from the edge |
| `castle_kc` | Caddy, Keycloak and its database, every oauth2-proxy | Keycloak's own traffic and the proxies' calls to it |
| `castle_tenant_<codename>` | that tenant's oauth2-proxy, castle and Postgres | one tenant's application and data |

Only Caddy publishes ports (80 and 443), and BIND when it is used (53). A
castle and its database are on their tenant's network and nowhere else.

## A request

```mermaid
sequenceDiagram
  participant B as Browser
  participant C as Caddy
  participant P as oauth2-proxy
  participant A as castle
  participant D as Postgres
  B->>C: GET /api/projects, with the session cookie
  C->>P: the request, X-Forwarded-Uri removed, X-Real-IP set
  alt no valid session for this codename
    P-->>B: redirect to the realm's sign-in page
  else valid session
    P->>A: the request, with the identity headers set by the proxy
    A->>D: the user by email, role from the groups, status
    A-->>B: what this user may see, and nothing else
  end
```

Castle takes the user's email, name and groups from the headers the proxy
sets, on every request. The groups give the role (`castle-managers`,
`castle-staff`, anything else is a client); the project memberships that
management keeps in castle give the rest. The rules are in the
[README](../README.md#access-control).

## A sign-in

```mermaid
sequenceDiagram
  participant B as Browser
  participant P as oauth2-proxy
  participant K as Keycloak
  B->>P: any page, no session
  P-->>B: redirect to the codename's realm, with a challenge and a nonce
  B->>K: sign in
  K-->>B: redirect back with a one-time code
  B->>P: the code
  P->>K: code, client secret and the challenge's verifier, over the internal network
  K-->>P: tokens
  P-->>B: session cookie, encrypted, for this codename only
  loop every 4 minutes while the portal is used
    P->>K: refresh
    K-->>P: new tokens, or a refusal that ends the session
  end
```

The code is bound to the browser that started the sign-in: Keycloak redeems
it only for the verifier the proxy kept for that browser (PKCE), and the proxy
accepts the tokens only with the nonce it sent along.

A session lasts at most ten hours. Because it is refreshed with Keycloak every
four minutes, disabling a user there, ending their session or changing their
groups reaches the portal within that time. Sign-out ends the Keycloak session
as well.

## Security model

What is protected: the findings, drafts above all, with their discussion and
images; which codenames belong to clients; credentials and sessions.

### Boundaries

1. **Internet to edge.** Everything from outside is untrusted. Caddy serves
   the pool's certificates, which were issued in advance, and forwards a
   request to its codename's proxy. It removes the header a visitor could use
   to tell the proxy which path is being asked for, and sets the visitor's
   address itself.
2. **Edge to proxy.** oauth2-proxy lets a request through only with a valid
   session for that codename's realm. Every codename has its own realm, client
   secret and cookie secret, so a session for one is nothing to another.
3. **Proxy to castle.** Castle believes the identity headers. That is sound
   only because nothing but its own proxy can reach it: it publishes no port
   and shares a network with that proxy and its database alone. This is the
   rule the rest leans on; `CASTLE_PROXY_SECRET` adds a secret the proxy must
   present, for deployments that want a second check.
4. **Castle to data.** Authorization is decided per request in the handlers.
   Someone who is not on a project gets 404 for everything in it.
5. **The host.** The host, its operator and Keycloak's administrators are
   trusted. Tenants share a kernel.

### Threats and answers

| Threat | Answer |
|--------|--------|
| Identity headers forged from outside | castle is reachable through its proxy only; the proxy sets the identity headers itself; the edge removes `X-Forwarded-Uri` |
| One client reading another's findings | per tenant: its own castle, database, network, realm and secrets; within a tenant: membership per project |
| A client reading drafts | drafts and their comments go to project staff only: a manager, or someone who is staff both to Keycloak and on that project |
| Access outliving the person's role | the role is read on every request, the session is refreshed with Keycloak every four minutes and ends after ten hours, a disabled account is refused at once, and management removes memberships |
| Guessed passwords | temporary lockout in Keycloak, no self-registration, no self-service password reset; the built-in login of `jwt` mode is throttled |
| Script in a finding or comment | Markdown goes through a sanitizer, diagrams render in mermaid's strict mode, and the Content-Security-Policy allows scripts from the portal's origin only. The text itself is stored as written, since a finding may quote a payload |
| A hostile upload | the type is decided by the file's first bytes (PNG, JPEG, GIF, WebP), 10 MiB at most, stored under a random name, served with `nosniff` to signed-in users |
| Finding out which codenames are in use | tenants and canaries answer alike at the edge; certificates for the whole pool are issued and renewed in one batch; promoting a canary changes neither certificate nor DNS; a request by address or unknown name gets a neutral certificate; the bundled DNS allows no zone transfer |
| A sign-in code that reaches someone else | it is of no use in another browser: Keycloak redeems it only for the verifier kept for the browser that asked (PKCE), and the proxy accepts the tokens only with that browser's nonce; a code is used once and lives a minute |
| A stolen backup | bundles are encrypted on the host to a public key whose private half is kept elsewhere |
| An instance serving with a broken configuration | the settings are parsed strictly at start, `DATABASE_URL` and `JWT_SECRET` have no default, and a logo that cannot be served stops the start |
| A tampered dependency or image | lockfiles; `cargo audit`, `npm audit`, Trivy, Semgrep and TruffleHog on every pull request and weekly; CI builds the image with provenance and an SBOM; `castlectl upgrade` resolves the image to a digest once, so every instance runs the same build |

### Limits

- Keycloak is trusted for identity: whoever controls it can sign in as anyone
  in that deployment.
- Tenants are separated by containers and networks on one kernel, not by
  virtual machines.
- One host is one copy of everything. The backups are what survives it.
