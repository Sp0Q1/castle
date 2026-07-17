import { type FormEvent, useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { api } from "../api/client";
import type { Project } from "../api/types";
import { useAuth } from "../auth/AuthContext";

export function ProjectsPage() {
  const { user } = useAuth();
  const [projects, setProjects] = useState<Project[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [formError, setFormError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const isManager = user?.role === "manager";

  const load = () => {
    setLoading(true);
    api
      .listProjects()
      .then(setProjects)
      .catch((err) => setError(err instanceof Error ? err.message : "Failed to load"))
      .finally(() => setLoading(false));
  };

  useEffect(load, []);

  const onCreate = async (e: FormEvent) => {
    e.preventDefault();
    setFormError(null);
    setBusy(true);
    try {
      await api.createProject(name, description.trim() ? description : null);
      setName("");
      setDescription("");
      load();
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "Failed to create project");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="stack">
      <div className="page-head">
        <h1>Projects</h1>
        <p className="muted">
          {isManager
            ? "Open engagements and onboard staff and clients."
            : "Engagements you have been onboarded to."}
        </p>
      </div>

      {isManager && (
        <form className="card" onSubmit={onCreate}>
          <h2>New project</h2>
          <label>
            Name
            <input value={name} onChange={(e) => setName(e.target.value)} required />
          </label>
          <label>
            Description
            <textarea
              value={description}
              rows={2}
              onChange={(e) => setDescription(e.target.value)}
            />
          </label>
          {formError && <div className="alert">{formError}</div>}
          <button type="submit" className="btn btn-primary" disabled={busy}>
            {busy ? "Creating…" : "Create project"}
          </button>
        </form>
      )}

      {loading && <p className="muted">Loading…</p>}
      {error && <div className="alert">{error}</div>}
      {!loading && !error && projects.length === 0 && (
        <p className="muted">No projects yet.</p>
      )}

      <ul className="list">
        {projects.map((p) => (
          <li key={p.id} className="card list-item">
            <div>
              <Link to={`/projects/${p.id}`} className="list-title">
                {p.name}
              </Link>
              {p.description && <p className="muted">{p.description}</p>}
            </div>
            <span className={`badge status-${p.status}`}>{p.status}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
