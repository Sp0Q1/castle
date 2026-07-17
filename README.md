# castle

A [loco.rs](https://loco.rs) (v0.16) JSON API for a security **reporting company**:
management opens projects, onboards staff and clients, staff write findings, and
clients read the published findings and comment on them.

## Domain model

```
users (+ role: manager | staff | client)
  │  1─* projects            (projects.created_by → users.id)
  │  1─* project_members      (the onboarding join)
  │  1─* findings             (findings.author_id → users.id)
  │  1─* comments             (comments.user_id → users.id)
  │
projects
  │  1─* project_members  ── user_id → users     (staff & clients onboarded here)
  │  1─* findings
  │
findings (title, description, technical_description, impact, recommendation,
  │       severity, status: draft | published)
  │  1─* comments
  │
comments (body)
```

| Table | Purpose | Key columns |
|-------|---------|-------------|
| `users` | People. `role` decides platform-wide capability. | `role`, `email`, `name`, `pid` |
| `projects` | Engagement opened by management. | `name`, `description`, `status`, `created_by` |
| `project_members` | Onboards a user into a project as `staff` or `client`. Unique per (project, user). | `project_id`, `user_id`, `role` |
| `findings` | A report item with the four required sections (markdown, with inline images). `finding_type` is a free-text classification; `severity` is `low`/`medium`/`elevated`/`high`/`extreme`. Visible to clients only when `published`. | `title`, `finding_type`, `description`, `technical_description`, `impact`, `recommendation`, `severity`, `status`, `project_id`, `author_id` |
| `comments` | Discussion on a finding (markdown). | `body`, `finding_id`, `user_id` |

Each domain file lives where loco expects it:
`migration/src/*` (schema), `src/models/_entities/*` (SeaORM entities),
`src/models/*` (model logic + `ActiveModelBehavior`), `src/controllers/*`
(HTTP + authorization), `src/views/*` (response shapes that never leak
credentials).

## Roles & access control

Enforced in the controllers (see `src/controllers/{projects,findings,comments}.rs`):

| Action | Who is allowed |
|--------|----------------|
| Create a project | `manager` |
| Onboard a member (staff or client) | `manager` |
| Write / edit / publish / unpublish / delete a finding | the finding's author, a project `staff` member, or a `manager` |
| List / read findings | managers & staff: all findings; **clients: only `published`** |
| Comment on a finding | any project member who can see the finding (so clients only on published findings) |
| View a project | managers (all), otherwise members of that project |

## Authentication & SSO

Castle has two auth modes, selected by `settings.auth_mode`:

- **`jwt`** (default; `development`) — the built-in email/password login issues a
  JWT. For local dev and tests.
- **`proxy`** (`production`) — Castle runs behind **oauth2-proxy**, which does the
  OIDC flow with **Keycloak** and forwards the identity as request headers.
  Castle handles no tokens/secrets: it provisions users on first sign-in and
  derives the platform role from Keycloak groups on every request. Self-service
  registration and the password endpoints are not mounted in this mode. Full
  setup + the critical network-isolation requirement are in
  [`deploy/oauth2-proxy-keycloak.md`](deploy/oauth2-proxy-keycloak.md).

Both modes resolve the request's user through one `CurrentUser` extractor
(`src/security.rs`), so the controllers are identical across modes.

**Onboarding a not-yet-registered person** works in either mode: onboarding by
email creates an `invited` placeholder user (plus their project membership); on
first login it is matched by email and flipped to `active`.

## API

All routes require a `Bearer` JWT (obtained from `/api/auth/login`) except the
auth endpoints themselves.

```
POST   /api/auth/register                         # { email, password, name }
POST   /api/auth/login                            # -> { token, ... }

POST   /api/projects                              # manager: { name, description? }
GET    /api/projects                              # projects visible to you
GET    /api/projects/{project_id}
POST   /api/projects/{project_id}/members         # manager: { user_email, role }
GET    /api/projects/{project_id}/members

POST   /api/projects/{project_id}/findings        # staff: { title, finding_type?,
                                                  #   description, technical_description,
                                                  #   impact, recommendation, severity? }
GET    /api/projects/{project_id}/findings        # clients see only published
GET    /api/findings/{finding_id}                 # finding + author + comments
PUT    /api/findings/{finding_id}                 # staff: partial update
POST   /api/findings/{finding_id}/publish         # staff: make visible to clients

GET    /api/findings/{finding_id}/comments
POST   /api/findings/{finding_id}/comments        # { body }

POST   /api/uploads                               # auth: multipart image -> { url }
GET    /api/uploads/{name}                         # public (unguessable name); serves the image
```

The long text fields (`description`, `technical_description`, `impact`,
`recommendation`, and comment `body`) are markdown. Images dropped/pasted into
the editor are uploaded via `POST /api/uploads` and embedded as
`![](/api/uploads/<name>)`. The serve endpoint is intentionally unauthenticated
(an `<img>` can't send the JWT) and relies on the unguessable name — for a real
deployment, move to signed URLs or a cookie-scoped check.

## Running it

The app's own binary (`castle-cli`) exposes every loco subcommand, so no global
CLI is needed — run them through `cargo run --`:

```bash
# from this directory — migrate, seed the demo users, and serve
cargo run -- db migrate
cargo run -- db seed        # seeds the demo accounts below
cargo run -- start          # serves on http://localhost:5150
```

`config/development.yaml` sets `auto_migrate: true`, so `cargo run -- start`
also applies migrations on boot. The database defaults to a local SQLite file
(`castle_development.sqlite`); override with the `DATABASE_URL` env var.

### Demo accounts (after `db seed`)

All three share the password **`12341234`**:

| Email | Password | Role |
|-------|----------|------|
| `manager@example.com` | `12341234` | manager |
| `staff@example.com` | `12341234` | staff |
| `client@example.com` | `12341234` | client |

> Optional: `cargo install loco` installs the standalone `loco`/`cargo loco`
> CLI (used for scaffolding and generators). With it you can also write
> `cargo loco start`, `cargo loco db migrate`, etc. It is not required to run
> this project.

### Example flow

```bash
# log in as the seeded manager (or register your own users first)
TOKEN=$(curl -s localhost:5150/api/auth/login \
  -H 'content-type: application/json' \
  -d '{"email":"manager@example.com","password":"<password>"}' | jq -r .token)

# open a project and onboard a staff member and a client
curl -s localhost:5150/api/projects -H "authorization: Bearer $TOKEN" \
  -H 'content-type: application/json' -d '{"name":"Acme Web App Pentest"}'
curl -s localhost:5150/api/projects/1/members -H "authorization: Bearer $TOKEN" \
  -H 'content-type: application/json' -d '{"user_email":"staff@example.com","role":"staff"}'
curl -s localhost:5150/api/projects/1/members -H "authorization: Bearer $TOKEN" \
  -H 'content-type: application/json' -d '{"user_email":"client@example.com","role":"client"}'

# as staff: write a finding, then publish it so the client can see it
# POST /api/projects/1/findings  ...  then  POST /api/findings/1/publish

# as the client: read published findings and comment
# GET  /api/projects/1/findings   (drafts are hidden)
# POST /api/findings/1/comments   {"body":"Thanks, we'll remediate this."}
```

## Web UI (frontend)

A React SPA in [`frontend/`](frontend/README.md) provides the UI on top of the
API: login/register, a projects list, a project view (members + findings, with
manager-only onboarding and staff-only finding authoring), and a finding view
(the four report sections, staff publish, and the comment thread). It is
role-aware — actions appear based on the signed-in user's role.

The project view shows two pie charts (findings by type and by severity). The
finding form uses a **markdown editor** (`@uiw/react-md-editor`) for the long
fields with **drag-and-drop / paste image upload**, a one-line **type** input
that suggests types already used on the project, and the low→extreme severity
scale. Findings and comments render markdown — bold/italic/lists/code and
**mermaid** diagrams (```mermaid fenced blocks). The editor and mermaid load
lazily so login and the projects list stay lightweight.

```bash
# dev: two processes — the API, and the frontend dev server (proxies /api)
cargo run -- start                       # terminal 1  (API on :5150)
npm --prefix frontend install            # once
npm --prefix frontend run dev            # terminal 2  (opens the app)
```

## Production

In production loco serves the built SPA and the API from one process
(`config/production.yaml` points the static middleware at `frontend/dist`):

```bash
npm --prefix frontend run build          # -> frontend/dist
JWT_SECRET="$(openssl rand -base64 48)" DATABASE_URL=... \
  cargo run --release -- start --environment production
```

**`JWT_SECRET` must be a base64 string** — loco base64-decodes it, so a value
containing `-`/`_`/other non-base64 characters makes token signing (and thus
login) fail. Generate one with `openssl rand -base64 48`.

