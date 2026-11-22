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
| `castle_kc` | Caddy, Keycloak, every oauth2-proxy | the proxies' calls to Keycloak |
| `castle_kcdb`, internal | Keycloak and its database | Keycloak's own data |
| `castle_tenant_<codename>`, internal | that tenant's oauth2-proxy, castle and Postgres | one tenant's application and data |

Only Caddy publishes ports (80 and 443), and BIND when it is used (53). A
castle and its database are on their tenant's network and nowhere else. An
internal network is closed both ways: no other network is routed to it, and
what sits on it reaches nothing outside.

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
    P->>A: the request, with the identity headers and the shared secret set by the proxy
    A->>D: the account bound to that person, role from the groups, status
    A-->>B: what this user may see, and nothing else
  end
```

Castle takes who is asking from the headers the proxy sets, on every request,
and only from a request that carries the secret the two share: Keycloak's id
for the person, which the account is bound to, their address, name and groups.
The groups give the role (`castle-managers`, `castle-staff`,
`castle-clients`; someone in none of them is refused); the project memberships
that management keeps in castle give the rest. The rules are in the
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
  P->>K: code, client secret and the challenge's verifier, inside the host
  K-->>P: tokens
  P-->>B: session cookie, encrypted, for this codename only
  loop every 4 minutes while the portal is used
    P->>K: refresh
    K-->>P: new tokens, or a refusal that ends the session
  end
```

The code is bound to the browser that started the sign-in: Keycloak starts no
sign-in for the client without a challenge and redeems the code only for the
verifier the proxy kept for that browser (PKCE), and the proxy accepts the
tokens only with the nonce it sent along.

A session lasts at most ten hours. Because it is refreshed with Keycloak every
four minutes, disabling a user there, ending their session or changing their
groups reaches the portal within that time. Sign-out ends the Keycloak session
as well.

## Several hosts, one overview

A host is complete in itself and shares nothing with another. What tells the
operator how all of them are is a page made on a machine of the operator's
own, from what each host says about itself.

```mermaid
flowchart LR
  subgraph host["A host"]
    app["castle of each tenant"]
    engine["containers, certificates, timers"]
    ctl["castlectl status, every five minutes"]
    file[("status.json")]
    account["account castle-status: prints the file, nothing else"]
  end
  subgraph own["The operator's machine"]
    overview["overview.py"]
    page["one static page"]
  end
  app -->|"summary: ids, counts, times"| ctl
  engine --> ctl --> file --> account
  overview -->|"ssh: pinned host key, a key for that account"| account
  overview --> page
  page -.->|"links"| portal(["a client's portal, signing in as always"])
```

A tenant's castle gives its host a summary: for each project its id, how many
findings there are by status and severity, and the comments of the last seven
days as times, ids and the role of who wrote them. `castlectl status` puts
those together with what it reads itself: which instances run and how, the
images, until when the served certificates are valid, the newest backup, the
host's disk and timers. No name, title or text is in that document, and none
is read to make it. Nothing listens for the overview: it asks, each host on its
own, and only reads ([`deploy/compose/docs/status.md`](../deploy/compose/docs/status.md),
[`deploy/overview/README.md`](../deploy/overview/README.md)).

## Security model

What is protected: the findings, drafts above all, with their discussion and
images; which codenames belong to clients; credentials and sessions.

### Boundaries

1. **Internet to edge.** Everything from outside is untrusted. Caddy serves
   the pool's certificates, which were issued in advance, and forwards a
   request to its codename's proxy. It removes the header a visitor could use
   to tell the proxy which path is being asked for, and sets the visitor's
   address itself. It answers the sign-out address to a browser that goes
   there, and not to a request a page makes for an image, a font or a
   stylesheet. Every answer of every codename leaves with the same
   security headers, and none that names the software behind it.
2. **Edge to proxy.** oauth2-proxy lets a request through only with a valid
   session for that codename's realm. Every codename has its own realm, client
   secret and cookie secret, so a session for one is nothing to another.
3. **Proxy to castle.** Castle believes the identity headers of a request that
   carries the secret it shares with its own proxy, and of no other. Every
   codename has its own secret; the proxy puts it on everything it forwards,
   in place of what the browser sent, and castle does not start without one.
   The network is the second lock: castle publishes no port, and its tenant's
   network is internal, joined by that proxy and the database alone.
4. **Castle to data.** Authorization is decided per request in the handlers.
   Someone who is not on a project gets 404 for everything in it.
5. **The host.** The host, its operator and Keycloak's administrators are
   trusted. Tenants share a kernel.
6. **Castle to host, host to overview.** Neither is taken at its word. The
   host reads at most a megabyte of what a tenant's castle prints and rebuilds
   the summary field by field; one that is not exactly a summary is left out.
   The status is put together in one place, which holds every value to what
   the overview takes and keeps the whole within what the overview reads:
   where it would not fit, the largest summaries are left out first, so no
   tenant crowds out the status of its host. The overview does the same with
   a host's status, and asks a host only after it has shown the host key that
   is pinned for it. The account it asks signs in with a key and can print
   the status and nothing else.

### Threats and answers

| Threat | Answer |
|--------|--------|
| Forged identity headers | castle reads them only from a request that carries the secret it shares with its own proxy; the proxy sets them itself, in place of what a browser sends; the tenant's network is internal and castle publishes no port; the edge removes `X-Forwarded-Uri` |
| A request that another site starts in a signed-in browser | the session cookie is `SameSite=Lax`, and castle refuses a changing request that the browser does not mark as coming from the portal's own pages |
| One client reading another's findings | per tenant: its own castle, database, network, realm and secrets; within a tenant: membership per project |
| A client reading drafts | drafts and their comments go to project staff only: a manager, or someone who is staff both to Keycloak and on that project |
| Access outliving the person's role | the role is read on every request, the session is refreshed with Keycloak every four minutes and ends after ten hours, a disabled account is refused at once, and management removes memberships |
| An address that changes hands, or a person who changes address | an account is bound to Keycloak's id for the person: a new holder of an old address is refused and gets nothing of the old account, and a disabled or anonymized person stays that under any address |
| A page that says one thing while the server does another | a manager is on no project and is listed for each one as a manager; only memberships can be changed or removed, with a confirmation; a membership that gives less than it reads is marked |
| Losing what someone wrote when they leave | a user is never deleted: they are anonymized, and the database refuses to delete a user that anything points at |
| Guessed passwords | temporary lockout in Keycloak, no self-registration, no self-service password reset; the built-in login of `jwt` mode is throttled |
| Keycloak's administration reached from outside | to the world the sign-in host serves the pool's realms and Keycloak's static files; the master realm, the admin console and the admin API answer only a request that starts on the server; once `castlectl keycloak-setup` has run, `castlectl` signs in as a service account with a secret and no password, and the master realm locks out after failed sign-ins |
| Script in a finding or comment | Markdown goes through a sanitizer, diagrams render in mermaid's strict mode, and the Content-Security-Policy denies everything by default: scripts come from the portal's own files only, with no inline script or handler. The text itself is stored as written, since a finding may quote a payload |
| A hostile upload | the type is decided by the file's first bytes (PNG, JPEG, GIF, WebP), 10 MiB at most, stored under a random name, served to signed-in users with `nosniff` and a policy of its own under which it loads and runs nothing; a text can show an upload as an image and nothing else |
| Finding out which codenames are in use | tenants and canaries answer alike at the edge; certificates for the whole pool are issued and renewed in one batch; promoting a canary changes neither certificate nor DNS; a request by address or unknown name gets a neutral certificate; the bundled DNS allows no zone transfer |
| A sign-in code that reaches someone else | it is of no use in another browser: Keycloak redeems it only for the verifier kept for the browser that asked (PKCE), and the proxy accepts the tokens only with that browser's nonce; a code is used once and lives a minute |
| A stolen backup | bundles are encrypted on the host to a public key whose private half is kept elsewhere |
| An instance serving with a broken configuration | the settings are parsed strictly at start, `DATABASE_URL` and `JWT_SECRET` have no default, proxy mode does not start without its proxy's secret, and a logo that cannot be served stops the start |
| A stolen status key | it reads one host's status and does nothing else there: the account runs one command whatever is asked, without terminal, forwarding or an environment of the other side's choosing, a connection that asks for nothing is closed, and all its connections together are held to a dozen processes. The status says which codenames are tenants and how much goes on in them, and carries no name, title or text. Deleting the key's line on the host ends it |
| A host that lies to the overview | it lies about itself alone: every host is asked on its own, an answer is read up to a megabyte and shown only if every field is what a status has there, links are built from a codename and the domain the overview was configured with, and a status older than fifteen minutes is shown as old. Why a host gave no status is what ssh reported, which a host that is spoken to can influence: every such host is one to look at |
| A client's text reaching the overview | none is read for it: a summary is ids, counts, times and words from a fixed list, the host drops one that has anything else in it, and the page has no script and a policy under which it loads nothing |
| A tampered dependency or image | lockfiles; `cargo audit`, `npm audit`, Trivy, Semgrep and TruffleHog on every pull request and weekly; CI builds the image with provenance and an SBOM; `castlectl upgrade` resolves the image to a digest once, so every instance runs the same build |

### Limits

- Keycloak is trusted for identity: whoever controls it can sign in as anyone
  in that deployment.
- Tenants are separated by containers and networks on one kernel, not by
  virtual machines.
- One host is one copy of everything. The backups are what survives it.
- The overview shows what each host says about itself. A host that is taken
  over can say that all is well.
