import {
  type ActionFunctionArgs,
  Link,
  type LoaderFunctionArgs,
  useFetcher,
  useLoaderData,
} from "react-router-dom";
import { api, errorMessage } from "../api/client";
import {
  type Finding,
  type Member,
  type Project,
  SEVERITIES,
} from "../api/types";
import { useUser } from "../auth/session";
import { Breadcrumbs } from "../components/Breadcrumbs";
import {
  FindingFields,
  findingParams,
  findingTypes,
} from "../components/FindingFields";
import { PieChart, type Slice } from "../components/PieChart";

const SEVERITY_COLORS: Record<string, string> = {
  low: "#7ee196",
  medium: "#f0cf6b",
  elevated: "#f5ac6b",
  high: "#ff8f6b",
  extreme: "#ff6b6b",
};
const TYPE_PALETTE = [
  "#4c8bf5",
  "#9b6dff",
  "#2dbfb0",
  "#f0cf6b",
  "#f5ac6b",
  "#ff6b6b",
  "#7ee196",
  "#b7c2cc",
];

const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

interface Data {
  project: Project;
  members: Member[];
  findings: Finding[];
}

export async function loader({ params }: LoaderFunctionArgs): Promise<Data> {
  const id = Number(params.id);
  const [project, members, findings] = await Promise.all([
    api.getProject(id),
    api.listMembers(id),
    api.listFindings(id),
  ]);
  return { project, members, findings };
}

export async function action({ params, request }: ActionFunctionArgs) {
  const id = Number(params.id);
  const form = await request.formData();
  try {
    if (form.get("intent") === "onboard") {
      await api.onboard(
        id,
        String(form.get("email")),
        String(form.get("role")),
      );
    } else {
      await api.createFinding(id, findingParams(form));
    }
  } catch (err) {
    return { error: errorMessage(err, "Failed to save") };
  }
  return null;
}

function ProjectDetailPage() {
  const { project, members, findings } = useLoaderData<Data>();
  const user = useUser();
  const newFinding = useFetcher<{ error: string }>();
  const onboard = useFetcher<{ error: string }>();

  const myMembership = members.find((m) => m.user.pid === user.pid);
  const isManager = user.role === "manager";
  const canWriteFindings = isManager || myMembership?.role === "staff";

  const severitySlices: Slice[] = SEVERITIES.map((sev) => ({
    label: cap(sev),
    value: findings.filter((f) => f.severity === sev).length,
    color: SEVERITY_COLORS[sev],
  }));

  const typeCounts = new Map<string, number>();
  for (const f of findings) {
    const key = f.finding_type.trim() || "Unspecified";
    typeCounts.set(key, (typeCounts.get(key) ?? 0) + 1);
  }
  const typeSlices: Slice[] = [...typeCounts.entries()].map(
    ([label, value], i) => ({
      label,
      value,
      color: TYPE_PALETTE[i % TYPE_PALETTE.length],
    }),
  );

  return (
    <div className="stack">
      <div className="page-head">
        <Breadcrumbs
          items={[{ label: "Projects", to: "/" }, { label: project.name }]}
        />
        <h1>
          {project.name}{" "}
          <span className={`badge status-${project.status}`}>
            {project.status}
          </span>
        </h1>
        {project.description && <p className="muted">{project.description}</p>}
      </div>

      <div className="charts">
        <PieChart title="Findings by type" slices={typeSlices} />
        <PieChart title="Findings by severity" slices={severitySlices} />
      </div>

      <div className="columns">
        <section className="stack">
          <h2>Findings</h2>
          {findings.length === 0 && (
            <p className="muted">
              {canWriteFindings
                ? "No findings yet."
                : "No published findings yet."}
            </p>
          )}
          <ul className="list">
            {findings.map((f) => (
              <li key={f.id} className="card list-item">
                <div>
                  <Link to={`/findings/${f.id}`} className="list-title">
                    {f.title}
                  </Link>
                  <div className="badges">
                    {f.finding_type.trim() && (
                      <span className="badge type-badge">{f.finding_type}</span>
                    )}
                    <span className={`badge sev-${f.severity}`}>
                      {f.severity}
                    </span>
                    <span className={`badge status-${f.status}`}>
                      {f.status}
                    </span>
                  </div>
                </div>
              </li>
            ))}
          </ul>

          {canWriteFindings && (
            // Keyed on the list it creates into, so a saved draft empties the form.
            <newFinding.Form
              className="card"
              method="post"
              key={findings.length}
            >
              <h3>New finding</h3>
              <FindingFields types={findingTypes(findings)} />
              {newFinding.data?.error && (
                <div className="alert">{newFinding.data.error}</div>
              )}
              <button
                type="submit"
                name="intent"
                value="finding"
                className="btn btn-primary"
                disabled={newFinding.state !== "idle"}
              >
                Save draft
              </button>
            </newFinding.Form>
          )}
        </section>

        <aside className="stack">
          <h2>Members</h2>
          <ul className="list">
            {members.map((m) => (
              <li key={m.id} className="card list-item">
                <div>
                  <span className="list-title">{m.user.name}</span>
                  <p className="muted">{m.user.email}</p>
                </div>
                <span className={`badge role-${m.role}`}>{m.role}</span>
              </li>
            ))}
          </ul>

          {isManager && (
            <onboard.Form className="card" method="post" key={members.length}>
              <h3>Onboard a member</h3>
              <label>
                User email
                <input type="email" name="email" required />
              </label>
              <label>
                Role
                {/* Defaults to the least-privileged role: a mis-onboard should grant
                    read-only client access, not staff write/publish rights. */}
                <select name="role" defaultValue="client">
                  <option value="staff">staff</option>
                  <option value="client">client</option>
                </select>
              </label>
              {onboard.data?.error && (
                <div className="alert">{onboard.data.error}</div>
              )}
              <button
                type="submit"
                name="intent"
                value="onboard"
                className="btn btn-primary"
                disabled={onboard.state !== "idle"}
              >
                Onboard
              </button>
            </onboard.Form>
          )}
        </aside>
      </div>
    </div>
  );
}

export { ProjectDetailPage as Component };
