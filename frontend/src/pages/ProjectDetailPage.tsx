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
  type UserSummary,
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
  managers: UserSummary[];
  members: Member[];
  findings: Finding[];
}

export async function loader({ params }: LoaderFunctionArgs): Promise<Data> {
  const id = Number(params.id);
  const [project, managers, members, findings] = await Promise.all([
    api.getProject(id),
    api.listManagers(id),
    api.listMembers(id),
    api.listFindings(id),
  ]);
  return { project, managers, members, findings };
}

export async function action({ params, request }: ActionFunctionArgs) {
  const id = Number(params.id);
  const form = await request.formData();
  try {
    switch (form.get("intent")) {
      case "onboard":
        await api.onboard(
          id,
          String(form.get("email")),
          String(form.get("role")),
        );
        break;
      case "set-role":
        await api.setMemberRole(
          id,
          Number(form.get("member")),
          String(form.get("role")),
        );
        break;
      case "remove-member":
        await api.removeMember(id, Number(form.get("member")));
        break;
      default:
        await api.createFinding(id, findingParams(form));
    }
  } catch (err) {
    return { error: errorMessage(err, "Failed to save") };
  }
  return null;
}

/** One member of the project; management can change their capacity or remove them. */
function MemberRow({ member, manage }: { member: Member; manage: boolean }) {
  const change = useFetcher<{ error: string }>();
  const busy = change.state !== "idle";
  const other = member.role === "staff" ? "client" : "staff";

  return (
    <li className="card list-item">
      <div>
        <span className="list-title">{member.user.name}</span>
        <p className="muted">{member.user.email}</p>
        {member.user.role === "manager" && (
          <p className="muted">
            Now a manager, who sees every project whatever this says.
          </p>
        )}
        {member.role === "staff" &&
          member.access === "client" &&
          member.user.status === "active" && (
            <p className="muted">
              Not staff to the identity provider, so on this project they can do
              what a client can.
            </p>
          )}
        {manage && (
          <change.Form method="post" className="actions">
            <input type="hidden" name="member" value={member.id} />
            <input type="hidden" name="role" value={other} />
            <button
              type="submit"
              name="intent"
              value="set-role"
              className="btn btn-ghost btn-small"
              disabled={busy}
              onClick={(e) => {
                if (
                  !window.confirm(
                    other === "staff"
                      ? `Make ${member.user.email} staff on this project? Staff read drafts and write, publish and delete findings.`
                      : `Make ${member.user.email} a client on this project? A client reads published findings only.`,
                  )
                ) {
                  e.preventDefault();
                }
              }}
            >
              Make {other}
            </button>
            <button
              type="submit"
              name="intent"
              value="remove-member"
              className="btn btn-danger btn-small"
              disabled={busy}
              onClick={(e) => {
                if (
                  !window.confirm(
                    `Remove ${member.user.email} from this project?`,
                  )
                ) {
                  e.preventDefault();
                }
              }}
            >
              Remove
            </button>
          </change.Form>
        )}
        {change.data?.error && <div className="alert">{change.data.error}</div>}
      </div>
      <span className={`badge role-${member.role}`}>{member.role}</span>
    </li>
  );
}

function ProjectDetailPage() {
  const { project, managers, members, findings } = useLoaderData<Data>();
  const user = useUser();
  const newFinding = useFetcher<{ error: string }>();
  const onboard = useFetcher<{ error: string }>();

  const isManager = user.role === "manager";
  const canWriteFindings = project.access === "staff";

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
          <h2>Who can open this project</h2>
          <p className="muted">
            Managers open every project. Everyone else is here because a manager
            put them on this one.
          </p>
          <ul className="list">
            {managers.map((manager) => (
              <li key={manager.id} className="card list-item">
                <div>
                  <span className="list-title">{manager.name}</span>
                  <p className="muted">{manager.email}</p>
                </div>
                <span className="badge role-manager">manager</span>
              </li>
            ))}
            {members.map((m) => (
              <MemberRow key={m.id} member={m} manage={isManager} />
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
