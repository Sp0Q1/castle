import {
  createBrowserRouter,
  Link,
  Outlet,
  redirect,
  useLoaderData,
  useNavigate,
  useNavigation,
  useRouteError,
} from "react-router-dom";
import { errorMessage } from "./api/client";
import type { AuthMode, CurrentUser } from "./api/types";
import { redirectIfSignedIn, requireUser, signOut } from "./auth/session";

function Layout() {
  const user = useLoaderData<CurrentUser>();
  const { state } = useNavigation();
  const navigate = useNavigate();
  return (
    <>
      <header className="navbar fixed-top">
        <div className="container">
          <Link to="/" className="brand">
            🏰 Castle
          </Link>
          <nav className="navbar-nav">
            <span className="whoami">
              {user.name}
              <span className={`badge role-${user.role}`}>{user.role}</span>
            </span>
            <button
              type="button"
              className="btn btn-ghost"
              onClick={() => signOut(user, navigate)}
            >
              Sign out
            </button>
          </nav>
        </div>
      </header>
      <main className="container page" aria-busy={state === "loading"}>
        <Outlet />
      </main>
    </>
  );
}

function RouteError() {
  const error = useRouteError();
  return <div className="alert">{errorMessage(error)}</div>;
}

/**
 * The pages are `lazy` so the markdown editor and mermaid land in chunks that are
 * only fetched when a page that uses them is opened.
 *
 * In proxy mode Keycloak owns signing in, so the built-in forms are not routed at
 * all — the root loader hands unauthenticated visitors to oauth2-proxy instead.
 */
export function createRouter(mode: AuthMode) {
  return createBrowserRouter([
    {
      id: "root",
      path: "/",
      loader: requireUser,
      element: <Layout />,
      errorElement: <RouteError />,
      hydrateFallbackElement: (
        <main className="container page">
          <p className="muted">Loading…</p>
        </main>
      ),
      children: [
        {
          index: true,
          lazy: () => import("./pages/ProjectsPage"),
          errorElement: <RouteError />,
        },
        {
          path: "projects/:id",
          lazy: () => import("./pages/ProjectDetailPage"),
          errorElement: <RouteError />,
        },
        {
          path: "findings/:id",
          lazy: () => import("./pages/FindingDetailPage"),
          errorElement: <RouteError />,
        },
      ],
    },
    ...(mode === "jwt"
      ? [
          {
            path: "/login",
            loader: redirectIfSignedIn,
            lazy: () => import("./pages/LoginPage"),
          },
          {
            path: "/register",
            loader: redirectIfSignedIn,
            lazy: () => import("./pages/RegisterPage"),
          },
        ]
      : []),
    { path: "*", loader: () => redirect("/") },
  ]);
}
