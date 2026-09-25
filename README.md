# castle

A [loco.rs](https://loco.rs) JSON API and React SPA for a security **reporting
company**: management opens projects, onboards staff and clients, staff write
findings, and clients read the published findings and comment on them.

## Domain model

```
users (role: manager | staff | client; status: invited | active)
  │  1─* projects            created_by
  │  1─* project_members     the onboarding join (role: staff | client)
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

Enforced in the controllers. A caller who is not a member of a project gets
**404**, not 401 or 403, so the existence of a project cannot be probed; a
member without the right role gets 403.

| Action | Who |
|--------|-----|
| Create a project, onboard a member | `manager` |
| Write, edit, publish, unpublish, delete a finding | the author, a project `staff` member, or a `manager` |
| Read findings | managers and staff: all; **clients: only `published`** |
| Comment | any project member who can see the finding |
| View a project | managers; otherwise members of that project |

## Authentication

Selected by `settings.auth_mode`; the settings block is parsed strictly and a
malformed one refuses to boot rather than falling back.

- **`jwt`** (development, tests): built-in email/password login issuing a JWT,
  plus verification, password reset and magic-link endpoints.
- **`proxy`** (production): castle runs behind **oauth2-proxy**, which does the
  OIDC flow with **Keycloak** and forwards identity as request headers. Castle
  handles no credentials: users are provisioned on first sign-in and the role
  is derived from Keycloak groups on every request. Only `/api/auth/current`
  and `/api/auth/mode` are mounted. See
  [`deploy/oauth2-proxy-keycloak.md`](deploy/oauth2-proxy-keycloak.md),
  including the network-isolation requirement.
- **`honeypot`**: the same auth paths as `jwt`, but every handler captures the
  attempt instead of authenticating. Used by decoy instances.

All modes resolve the request's user through one `CurrentUser` extractor in
`src/security.rs`, so controllers are identical across modes. Onboarding a
not-yet-registered person by email creates an `invited` user that becomes
`active` on first login.

## API

Everything except the auth endpoints requires an authenticated user (a
`Bearer` JWT in `jwt` mode, the proxy headers in `proxy` mode).

```
GET    /api/auth/mode                              # which auth mode is running
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

POST   /api/projects/{project_id}/findings         # staff
GET    /api/projects/{project_id}/findings         # clients see only published
GET    /api/findings/{finding_id}                  # finding + author + comments
PUT    /api/findings/{finding_id}                  # partial update
DELETE /api/findings/{finding_id}
POST   /api/findings/{finding_id}/publish
POST   /api/findings/{finding_id}/unpublish

GET    /api/findings/{finding_id}/comments
POST   /api/findings/{finding_id}/comments         # { body }

POST   /api/uploads                                # multipart image -> { url }
GET    /api/uploads/{name}                         # unauthenticated; unguessable name
```

The long fields and comment bodies are markdown; images pasted into the editor
go through `POST /api/uploads` and are embedded as `![](/api/uploads/<name>)`.
The serve endpoint is unauthenticated because an `<img>` cannot send a token,
and relies on the unguessable name.

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
points at one; CI runs both.

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
preparation, DNS, backups and admin rotation each have a page in
[`deploy/compose/docs/`](deploy/compose/docs/).
