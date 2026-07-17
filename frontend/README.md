# Castle — frontend

A React + TypeScript single-page app for the Castle reporting portal, built with
[Rsbuild](https://rsbuild.dev/). It talks to the loco JSON API under `/api` and
is role-aware (managers, staff, clients see different actions).

## Stack

- **React 19** + **react-router-dom** (routing)
- **Rsbuild** (bundler) — `rsbuild.config.ts` proxies `/api` to the loco server on `127.0.0.1:5150` in dev
- **@uiw/react-md-editor** — markdown editor + renderer for the long fields and comments
- **mermaid** — diagram rendering for ```mermaid fenced code blocks (lazy-loaded)
- **TypeScript** (type-checked with `npx tsc --noEmit`)
- **Biome** (`npm run lint`)

## Notable components (`src/components/`)

- `MarkdownField` — the editor, with drag-and-drop / paste image upload (`POST /api/uploads`)
- `Markdown` — read-only renderer (GFM + mermaid via a custom `code` component)
- `Mermaid` — renders one diagram (lazy-imports mermaid, initializes once)
- `PieChart` — inline-SVG pie for the type/severity breakdowns
- `TypeInput` — one-line type field with a datalist of the project's existing types

The project and finding pages are `React.lazy`-loaded so the heavy editor/mermaid
chunks only download when you open them.

## Layout

```
src/
  index.tsx              entry
  index.css              styles (dark theme)
  api/
    types.ts             TS types mirroring the API responses
    client.ts            fetch wrapper (attaches the JWT) + typed endpoints
  auth/AuthContext.tsx   token storage + current user + login/logout
  App.tsx                router, header, protected routes
  pages/
    LoginPage.tsx  RegisterPage.tsx
    ProjectsPage.tsx        list + (manager) create
    ProjectDetailPage.tsx   members + findings + (manager) onboard + (staff) new finding
    FindingDetailPage.tsx   the four report sections + (staff) publish + comments
```

## Develop

```sh
npm install
npm run dev          # rsbuild dev server (HMR), proxies /api -> loco:5150
```

Run the API alongside it from the repo root: `cargo run -- start`.
Then open the dev server URL and sign in.

## Build for production

```sh
npm run build        # -> frontend/dist
```

`config/production.yaml` serves `frontend/dist` at `/` (with an `index.html`
fallback so client routes like `/projects/1` resolve to the SPA), so in
production loco serves both the API and the app from one process.
