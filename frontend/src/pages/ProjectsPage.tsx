import {
  type ActionFunctionArgs,
  Link,
  useFetcher,
  useLoaderData,
} from "react-router-dom";
import { api, errorMessage } from "../api/client";
import type { Project } from "../api/types";
import { useUser } from "../auth/session";

export function loader(): Promise<Project[]> {
  return api.listProjects();
}

export async function action({ request }: ActionFunctionArgs) {
  const form = await request.formData();
  try {
    await api.createProject(
      String(form.get("name")),
      String(form.get("description")).trim() || null,
    );
  } catch (err) {
    return { error: errorMessage(err, "Failed to create project") };
  }
  return null;
}

function ProjectsPage() {
  const projects = useLoaderData<Project[]>();
  const isManager = useUser().role === "manager";
  const fetcher = useFetcher<{ error: string }>();
  const busy = fetcher.state !== "idle";

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
        // Keyed on the list it creates into, so a successful create empties the form.
        <fetcher.Form className="card" method="post" key={projects.length}>
          <h2>New project</h2>
          <label>
            Name
            <input name="name" required />
          </label>
          <label>
            Description
            <textarea name="description" rows={2} />
          </label>
          {fetcher.data?.error && (
            <div className="alert">{fetcher.data.error}</div>
          )}
          <button type="submit" className="btn btn-primary" disabled={busy}>
            {busy ? "Creating…" : "Create project"}
          </button>
        </fetcher.Form>
      )}

      {projects.length === 0 && <p className="muted">No projects yet.</p>}

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

export { ProjectsPage as Component };
