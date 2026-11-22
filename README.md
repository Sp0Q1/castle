# castle

A [loco.rs](https://loco.rs) JSON API and React SPA for a security **reporting
company**: management opens projects, onboards staff and clients, staff write
findings, and clients read the published findings and comment on them.

How the pieces fit in a deployment, how a request and a sign-in travel through
them, and what the design defends against:
[`docs/architecture.md`](docs/architecture.md).

## Domain model

```
users (role: manager | staff | client; status: invited | active | disabled)
  │  1─* projects            created_by
  │  1─* project_members     who is on a project and as what (role: staff |
  │                          client)
  │  1─* findings            author_id
  │  1─* comments            user_id
projects  1─* project_members, 1─* findings
findings  (title, finding_type, description, technical_description, impact,
          recommendation — markdown with inline images;
          severity: low | medium | elevated | high | extreme;
          status: draft | published)  1─* comments
```

Each domain file lives where loco expects it: `migration/src/*` (schema),
`src/models/_entities/*` (SeaORM entities), `src/models/*` (model logic),
`src/controllers/*` (HTTP + authorization), `src/views/*` (response shapes that
never leak credentials).

## Access control

Enforced in the controllers. Someone who cannot open a project gets **404**
for it and for everything in it, not 401 or 403, so the existence of a project
cannot be probed; someone who can open it and lacks the right for an action
gets 403. Changing a project's members is management's alone and answers 403 to
everyone else, whichever project is named.

| Action | Who |
|--------|-----|
| Create a project; onboard, change or remove a member | `manager` |
| Write, edit, publish, unpublish, delete a finding | a `manager`, or a `staff` user onboarded to the project as staff |
| Read findings | those who may write them: all; **every other member: only `published`** |
| Comment | anyone who can see the finding |
| View a project | managers; otherwise members of that project |
| See everyone with an account, their role and their projects | `manager` |

The role comes from the IdP and is read on every request. The membership is
management's statement about one project, and only management changes it. A
manager is on no project: the role opens all of them, so each project lists
its managers next to its members.

What someone may do on a project is worked out in one place, `Access` in
`src/models/project_members.rs`, from those two facts. Every handler asks it,
and a project and each member row carry the answer (`access`: `staff` or
`client`), so the pages render from it and never work the rule out again.
`tests/requests/policy.rs` states what each kind of person gets from every
address and fails for an address without a stated rule.

## Authentication

Selected by `settings.auth_mode`. The settings block is parsed strictly when
castle starts, and a malformed one stops it rather than falling back to a
default.

- **`jwt`** (development, tests): built-in email/password login issuing a JWT,
  plus verification, password reset and magic-link endpoints.
- **`proxy`** (every deployment): castle runs behind **oauth2-proxy**, which
  does the OIDC flow with **Keycloak** and forwards the identity as request
  headers. Castle handles no credentials: a user is provisioned on first
  sign-in and bound to the id Keycloak has for that person, so an account
  follows the person and not an address. The role is read from their Keycloak
  groups on every request, so a change there reaches castle when the proxy
  next renews the session with Keycloak, within four minutes. Someone in none
  of the three groups is refused. Only `/api/auth/current` and
  `/api/auth/mode` are mounted. Castle reads those headers only from a request
  that carries the secret it shares with its proxy, and does not start in this
  mode without one. They are read strictly (one value each, a group by its
  exact name), and a request that changes something has to come from the
  portal's own pages: see
  [`deploy/oauth2-proxy-keycloak.md`](deploy/oauth2-proxy-keycloak.md).

Both modes resolve the request's user through one `CurrentUser` extractor in
`src/security.rs`, so controllers are identical across modes. Onboarding
someone who has not signed in yet creates an `invited` user, which in `proxy`
mode becomes `active` the first time that person arrives. A `disabled` user is
refused on every request, in both modes, whatever session they still hold.

## API

Apart from signing in itself, `/api/auth/mode`, the logo and the three health
addresses, everything requires an authenticated user (a `Bearer` JWT in `jwt`
mode, the proxy headers in `proxy` mode). The pages' own files are served to
anyone who reaches castle.

```
GET    /api/auth/mode                              # the auth mode, and the logo if one is set
GET    /api/auth/current
POST   /api/auth/register | login | forgot | reset # jwt mode only
GET    /api/auth/verify/{token}                    # jwt mode only
POST   /api/auth/magic-link                        # jwt mode only
GET    /api/auth/magic-link/{token}                # jwt mode only
POST   /api/auth/resend-verification-mail          # jwt mode only

POST   /api/projects                               # manager
GET    /api/projects                               # projects visible to you
GET    /api/projects/{project_id}
POST   /api/projects/{project_id}/members          # manager: { user_email, role }
GET    /api/projects/{project_id}/members
PUT    /api/projects/{project_id}/members/{member_id}     # manager: { role }
DELETE /api/projects/{project_id}/members/{member_id}     # manager
GET    /api/projects/{project_id}/managers                # who opens it without being on it

POST   /api/projects/{project_id}/findings         # staff
GET    /api/projects/{project_id}/findings         # clients see only published
GET    /api/findings/{finding_id}                  # finding + author + comments
PUT    /api/findings/{finding_id}                  # partial update
DELETE /api/findings/{finding_id}
POST   /api/findings/{finding_id}/publish
POST   /api/findings/{finding_id}/unpublish

GET    /api/findings/{finding_id}/comments
POST   /api/findings/{finding_id}/comments         # { body }

GET    /api/users                                  # manager: everyone with an account

POST   /api/uploads                                # multipart image -> { url }
GET    /api/uploads/{name}                         # authenticated, like the rest

GET    /api/brand/logo                             # the deployment's logo, when configured

GET    /_ping | /_health | /_readiness             # loco's own: alive, and whether the database answers
```

The long fields and comment bodies are markdown; images pasted into the editor
go through `POST /api/uploads` and are embedded as `![](/api/uploads/<name>)`.
Serving an upload requires authentication too. An `<img>` cannot send a token,
so the SPA fetches each image through its API client and renders it from an
object URL instead of pointing the browser at the endpoint. Only the path of
an upload is an image to the pages: any other image reference in a text is
shown as its caption, and nothing is requested for it.

## Configuration and tasks

`config/<environment>.yaml`. In production these come from the environment:

| Variable | Meaning |
|----------|---------|
| `DATABASE_URL`, `JWT_SECRET` | required; without them castle does not start. `JWT_SECRET` is base64 |
| `DB_CONNECT_TIMEOUT`, `DB_IDLE_TIMEOUT`, `DB_MIN_CONNECTIONS`, `DB_MAX_CONNECTIONS` | the database pool: milliseconds and counts, with defaults of 5000, 30000, 1 and 10 |
| `CASTLE_AUTH_MODE` | `proxy` (the production default) or `jwt` |
| `CASTLE_PROXY_SECRET` | required in `proxy` mode: the secret castle shares with its proxy, which sends it with every request |
| `CASTLE_LOGOUT_URL` | where sign-out sends the browser |
| `CASTLE_LOGO` | path of an image shown instead of the built-in brand: PNG, JPEG, GIF, WebP or SVG, at most 1 MiB. One that cannot be served stops the server from starting |
| `HOST`, `PORT` | the portal's public origin, exactly as a browser names it (`https://portal.example.com`), and the listening port. In `proxy` mode a request that changes something has its `Origin` compared with `HOST` when the browser sends no `Sec-Fetch-Site` |
| `MAILER_HOST`, `MAILER_PORT`, `MAILER_SECURE` | the SMTP server for the mails of `jwt` mode; `proxy` mode sends none |

Accounts are managed from the command line of the instance. A user is never
deleted: `user:anonymize` replaces the name and address with a placeholder,
disables the account and takes it off every project, and what they wrote stays.
On Postgres the database refuses to delete a user that anything points at.
`user:unbind` lets an account go to whoever Keycloak next vouches for with its
address, for someone whose Keycloak account was made anew.

```bash
castle-cli task user:status email:someone@example.com status:disabled   # or status:active
castle-cli task user:anonymize email:someone@example.com                # for someone who left
castle-cli task user:unbind email:someone@example.com                   # or all:yes, after a move to another Keycloak realm
castle-cli task user:create email:… name:… password:…                   # jwt mode
```

In a deployment a task runs inside the tenant's container and names the
environment, or it reads the development configuration:
`podman exec castle-<codename> castle-cli task --environment production user:status …`.

## Running it locally

```bash
cargo run -- db migrate
cargo run -- db seed                     # demo accounts below
cargo run -- start                       # API on http://localhost:5150
npm --prefix frontend install && npm --prefix frontend run dev   # SPA, proxies /api
```

`config/development.yaml` uses SQLite (`castle_development.sqlite`) unless
`DATABASE_URL` is set, and applies migrations on boot. The seeded accounts
`manager@`, `staff@` and `client@example.com` all use the password `12341234`.

Tests run against SQLite by default and against Postgres when `DATABASE_URL`
points at one. CI runs them on Postgres, the engine every deployment uses.

## Frontend

[`frontend/`](frontend/README.md): React 19 + react-router data APIs, built
with Rsbuild. Role-aware projects and finding views, a markdown editor with
drag-and-drop image upload, mermaid diagrams in findings and comments. In
production loco serves the built SPA and the API from one process.

## Deployment

One VPS, Podman Compose, Caddy at the edge, one Keycloak realm per tenant, and
a pool of indistinguishable codename instances with certificates issued in
bulk. Start at [`deploy/compose/README.md`](deploy/compose/README.md); the
`castlectl` script there provisions, upgrades, backs up and restores. Host
preparation, DNS, upgrades and backups, moving and retiring a tenant, and
Keycloak's administrators each have a page in
[`deploy/compose/docs/`](deploy/compose/docs/). Sign-in, roles and sessions are
in [`deploy/oauth2-proxy-keycloak.md`](deploy/oauth2-proxy-keycloak.md).

Every push to `main` builds the image and publishes it as
`ghcr.io/sp0q1/castle:main`, with build provenance and an SBOM attached.
