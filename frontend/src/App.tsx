import { lazy, type ReactNode, Suspense } from "react";
import {
  BrowserRouter,
  Link,
  Navigate,
  Route,
  Routes,
  useNavigate,
} from "react-router-dom";
import { AuthProvider, useAuth } from "./auth/AuthContext";
import { LoginPage } from "./pages/LoginPage";
import { ProjectsPage } from "./pages/ProjectsPage";
import { RegisterPage } from "./pages/RegisterPage";

// The project and finding pages pull in the markdown editor (heavy), so load
// them on demand — login and the projects list stay lightweight.
const ProjectDetailPage = lazy(() =>
  import("./pages/ProjectDetailPage").then((m) => ({
    default: m.ProjectDetailPage,
  })),
);
const FindingDetailPage = lazy(() =>
  import("./pages/FindingDetailPage").then((m) => ({
    default: m.FindingDetailPage,
  })),
);

function ProtectedRoute({ children }: { children: ReactNode }) {
  const { user, loading } = useAuth();
  if (loading) {
    return <p className="muted">Loading…</p>;
  }
  if (!user) {
    return <Navigate to="/login" replace />;
  }
  return <>{children}</>;
}

function Header() {
  const { user, logout } = useAuth();
  const navigate = useNavigate();
  if (!user) {
    return null;
  }
  return (
    <header className="navbar fixed-top">
      <div className="container">
        <Link to="/" className="brand">
          🏰 Castle
        </Link>
        <nav className="navbar-nav">
          {user && (
            <span className="whoami">
              {user.name}
              <span className={`badge role-${user.role}`}>{user.role}</span>
            </span>
          )}
          <button
            type="button"
            className="btn btn-ghost"
            onClick={() => {
              logout();
              navigate("/login");
            }}
          >
            Sign out
          </button>
        </nav>
      </div>
    </header>
  );
}

export function App() {
  return (
    <BrowserRouter>
      <AuthProvider>
        <Header />
        <main className="container page">
          <Suspense fallback={<p className="muted">Loading…</p>}>
            <Routes>
              <Route path="/login" element={<LoginPage />} />
              <Route path="/register" element={<RegisterPage />} />
              <Route
                path="/"
                element={
                  <ProtectedRoute>
                    <ProjectsPage />
                  </ProtectedRoute>
                }
              />
              <Route
                path="/projects/:id"
                element={
                  <ProtectedRoute>
                    <ProjectDetailPage />
                  </ProtectedRoute>
                }
              />
              <Route
                path="/findings/:id"
                element={
                  <ProtectedRoute>
                    <FindingDetailPage />
                  </ProtectedRoute>
                }
              />
              <Route path="*" element={<Navigate to="/" replace />} />
            </Routes>
          </Suspense>
        </main>
      </AuthProvider>
    </BrowserRouter>
  );
}
