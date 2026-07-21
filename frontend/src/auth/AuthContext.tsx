import {
  createContext,
  type ReactNode,
  useContext,
  useEffect,
  useState,
} from "react";
import { api, setAuthToken } from "../api/client";
import type { CurrentUser } from "../api/types";

interface AuthState {
  token: string | null;
  user: CurrentUser | null;
  loading: boolean;
  login: (email: string, password: string) => Promise<void>;
  register: (email: string, password: string, name: string) => Promise<void>;
  logout: () => void;
}

const AuthContext = createContext<AuthState | null>(null);
const TOKEN_KEY = "castle.token";

export function AuthProvider({ children }: { children: ReactNode }) {
  const [token, setToken] = useState<string | null>(() =>
    localStorage.getItem(TOKEN_KEY),
  );
  const [user, setUser] = useState<CurrentUser | null>(null);
  const [loading, setLoading] = useState<boolean>(true);

  // On mount / token change, sync the token into the API client and load the
  // current user. We always ask `/api/auth/current`: in JWT mode it needs the
  // bearer token; in proxy mode there is no token and the identity comes from
  // the proxy's headers on every request — so a 200 there means we're signed in.
  useEffect(() => {
    setAuthToken(token);
    let active = true;
    setLoading(true);
    (async () => {
      try {
        const u = await api.currentUser();
        if (active) {
          setUser(u);
          setLoading(false);
        }
      } catch {
        if (!active) return;
        // Not authenticated. In proxy mode there is no local login form — hand
        // off to the IdP via oauth2-proxy (so we go to Keycloak, not a dead
        // form). In jwt mode, fall through to the built-in login page.
        try {
          const { mode } = await api.authMode();
          if (mode === "proxy") {
            const rd = encodeURIComponent(
              window.location.pathname + window.location.search,
            );
            window.location.href = `/oauth2/sign_in?rd=${rd}`;
            return; // leaving the page — keep the loading state, no form flash
          }
        } catch {
          // Mode unknown — behave like jwt and show the local form.
        }
        if (!active) return;
        setUser(null);
        // A stale JWT — drop it. (In proxy mode there is no token to clear.)
        if (token) {
          localStorage.removeItem(TOKEN_KEY);
          setAuthToken(null);
          setToken(null);
        }
        setLoading(false);
      }
    })();
    return () => {
      active = false;
    };
  }, [token]);

  const login = async (email: string, password: string) => {
    const res = await api.login(email, password);
    localStorage.setItem(TOKEN_KEY, res.token);
    setAuthToken(res.token);
    setToken(res.token);
  };

  const register = async (email: string, password: string, name: string) => {
    await api.register(email, password, name);
  };

  const logout = () => {
    if (token) {
      // JWT mode: drop the local token.
      localStorage.removeItem(TOKEN_KEY);
      setAuthToken(null);
      setToken(null);
      setUser(null);
    } else {
      // Proxy mode: sign out at oauth2-proxy (and, if configured, chain to the
      // IdP end-session endpoint via user.logout_url for a full RP-initiated logout).
      window.location.href = user?.logout_url || "/oauth2/sign_out?rd=/";
    }
  };

  return (
    <AuthContext.Provider value={{ token, user, loading, login, register, logout }}>
      {children}
    </AuthContext.Provider>
  );
}

export function useAuth(): AuthState {
  const ctx = useContext(AuthContext);
  if (!ctx) {
    throw new Error("useAuth must be used within an AuthProvider");
  }
  return ctx;
}
