import {
  type LoaderFunctionArgs,
  type NavigateFunction,
  redirect,
  redirectDocument,
  useRouteLoaderData,
} from "react-router-dom";
import { ApiError, api, setAuthToken } from "../api/client";
import type { AuthMode, CurrentUser } from "../api/types";

// jwt (dev) mode only — production runs proxy mode, where the session is an
// httpOnly oauth2-proxy cookie and no token is ever exposed to JavaScript.
// sessionStorage (per-tab, cleared on close) rather than localStorage narrows the
// window if script ever runs in this origin; the real defences are the markdown/
// upload sanitization and the CSP.
const TOKEN_KEY = "castle.token";

// Resolved once at startup and fixed for the life of the page. Loaders and
// actions run outside React, so this cannot live in component state.
let mode: AuthMode = "jwt";
let logo: string | null = null;

/** The deployment's own logo, when it has one; otherwise the built-in brand is shown. */
export const brandLogo = (): string | null => logo;

/** Resolves the server's auth mode and restores a jwt session. Call once, before routing. */
export async function initAuth(): Promise<AuthMode> {
  // Unknown (API unreachable): behave like jwt and show the local form.
  const about = await api.about().catch(() => null);
  mode = about?.mode ?? "jwt";
  logo = about?.logo ?? null;
  if (mode === "jwt") {
    setAuthToken(sessionStorage.getItem(TOKEN_KEY));
  }
  return mode;
}

function storeToken(token: string | null): void {
  if (token) {
    sessionStorage.setItem(TOKEN_KEY, token);
  } else {
    sessionStorage.removeItem(TOKEN_KEY);
  }
  setAuthToken(token);
}

export async function login(email: string, password: string): Promise<void> {
  storeToken((await api.login(email, password)).token);
}

export function signOut(user: CurrentUser, navigate: NavigateFunction): void {
  if (mode === "jwt") {
    storeToken(null);
    navigate("/login");
    return;
  }
  // Sign out at oauth2-proxy, chaining to the IdP's end-session endpoint when one
  // is configured (logout_url), for a full RP-initiated logout.
  window.location.href = user.logout_url || "/oauth2/sign_out?rd=/";
}

/**
 * Root loader: the signed-in user, or a redirect to wherever this deployment
 * signs people in. `/api/auth/current` answers for both modes — in jwt mode from
 * the bearer token, in proxy mode from the headers the proxy sets on every request.
 */
export async function requireUser({
  request,
}: LoaderFunctionArgs): Promise<CurrentUser> {
  try {
    return await api.currentUser();
  } catch (err) {
    // Signed in but refused (a disabled account). Signing in again would only
    // land back here, so let the route's error element say why instead.
    if (err instanceof ApiError && err.status === 403) {
      throw err;
    }
    if (mode === "proxy") {
      const { pathname, search } = new URL(request.url);
      const rd = encodeURIComponent(pathname + search);
      throw redirectDocument(`/oauth2/sign_in?rd=${rd}`);
    }
    storeToken(null); // whatever was there is stale
    throw redirect("/login");
  }
}

/** Guards the jwt-only login and register pages. */
export function redirectIfSignedIn(): Promise<Response | null> {
  return api.currentUser().then(
    () => redirect("/"),
    () => null,
  );
}

/** Every page below the root route has a user — its loader guarantees one. */
export const useUser = (): CurrentUser =>
  useRouteLoaderData("root") as CurrentUser;
