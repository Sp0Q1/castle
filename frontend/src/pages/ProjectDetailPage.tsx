import { type FormEvent, useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { api } from "../api/client";
import {
  type CreateFindingParams,
  type Finding,
  type Member,
  type Project,
  SEVERITIES,
} from "../api/types";
import { useAuth } from "../auth/AuthContext";
import { Breadcrumbs } from "../components/Breadcrumbs";
import { MarkdownField } from "../components/MarkdownField";
import { PieChart, type Slice } from "../components/PieChart";
import { TypeInput } from "../components/TypeInput";

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

const EMPTY_FINDING: CreateFindingParams = {
  title: "",
  finding_type: "",
  description: "",
  technical_description: "",
  impact: "",
  recommendation: "",
  severity: "medium",
};

const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

export function ProjectDetailPage() {
  const { id } = useParams();
  const projectId = Number(id);
  const { user } = useAuth();

  const [project, setProject] = useState<Project | null>(null);
  const [members, setMembers] = useState<Member[]>([]);
  const [findings, setFindings] = useState<Finding[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const [inviteEmail, setInviteEmail] = useState("");
  const [inviteRole, setInviteRole] = useState("staff");
  const [inviteError, setInviteError] = useState<string | null>(null);

  const [finding, setFinding] = useState<CreateFindingParams>(EMPTY_FINDING);
  const [findingError, setFindingError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const myMembership = members.find((m) => m.user.pid === user?.pid);
  const isManager = user?.role === "manager";
  const canWriteFindings = isManager || myMembership?.role === "staff";

  const load = () => {
    setLoading(true);
    Promise.all([
      api.getProject(projectId),
      api.listMembers(projectId),
      api.listFindings(projectId),
    ])
      .then(([p, m, f]) => {
        setProject(p);
        setMembers(m);
        setFindings(f);
      })
      .catch((err) => setError(err instanceof Error ? err.message : "Failed to load"))
      .finally(() => setLoading(false));
  };

  // biome-ignore lint/correctness/useExhaustiveDependencies: reload when the route id changes
  useEffect(load, [projectId]);

  const onInvite = async (e: FormEvent) => {
    e.preventDefault();
    setInviteError(null);
    setBusy(true);
    try {
      await api.onboard(projectId, inviteEmail, inviteRole);
      setInviteEmail("");
      load();
    } catch (err) {
      setInviteError(err instanceof Error ? err.message : "Failed to onboard");
    } finally {
      setBusy(false);
    }
  };

  const onCreateFinding = async (e: FormEvent) => {
    e.preventDefault();
    setFindingError(null);
    setBusy(true);
    try {
      await api.createFinding(projectId, finding);
      setFinding(EMPTY_FINDING);
      load();
    } catch (err) {
      setFindingError(err instanceof Error ? err.message : "Failed to create finding");
    } finally {
      setBusy(false);
    }
  };

  const set = (patch: Partial<CreateFindingParams>) =>
    setFinding((f) => ({ ...f, ...patch }));

  if (loading) return <p className="muted">Loading…</p>;
  if (error) return <div className="alert">{error}</div>;
  if (!project) return <p className="muted">Not found.</p>;

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
  const typeSlices: Slice[] = [...typeCounts.entries()].map(([label, value], i) => ({
    label,
    value,
    color: TYPE_PALETTE[i % TYPE_PALETTE.length],
  }));

  const typeSuggestions = [
    ...new Set(
      findings.map((f) => f.finding_type.trim()).filter((t): t is string => t.length > 0),
    ),
  ];

  return (
    <div className="stack">
      <div className="page-head">
        <Breadcrumbs items={[{ label: "Projects", to: "/" }, { label: project.name }]} />
        <h1>
          {project.name} <span className={`badge status-${project.status}`}>{project.status}</span>
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
              {canWriteFindings ? "No findings yet." : "No published findings yet."}
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
                    <span className={`badge sev-${f.severity}`}>{f.severity}</span>
                    <span className={`badge status-${f.status}`}>{f.status}</span>
                  </div>
                </div>
              </li>
            ))}
          </ul>

          {canWriteFindings && (
            <form className="card" onSubmit={onCreateFinding}>
              <h3>New finding</h3>
              <label>
                Title
                <input
                  value={finding.title}
                  onChange={(e) => set({ title: e.target.value })}
                  required
                />
              </label>
              <label>
                Type
                <TypeInput
                  value={finding.finding_type ?? ""}
                  onChange={(v) => set({ finding_type: v })}
                  suggestions={typeSuggestions}
                />
              </label>
              <label>
                Severity
                <select
                  value={finding.severity}
                  onChange={(e) => set({ severity: e.target.value })}
                >
                  {SEVERITIES.map((s) => (
                    <option key={s} value={s}>
                      {cap(s)}
                    </option>
                  ))}
                </select>
              </label>
              <div className="field">
                <span className="field-label">Description</span>
                <MarkdownField
                  value={finding.description}
                  onChange={(v) => set({ description: v })}
                />
              </div>
              <div className="field">
                <span className="field-label">Technical description</span>
                <MarkdownField
                  value={finding.technical_description}
                  onChange={(v) => set({ technical_description: v })}
                />
              </div>
              <div className="field">
                <span className="field-label">Impact</span>
                <MarkdownField value={finding.impact} onChange={(v) => set({ impact: v })} />
              </div>
              <div className="field">
                <span className="field-label">Recommendation</span>
                <MarkdownField
                  value={finding.recommendation}
                  onChange={(v) => set({ recommendation: v })}
                />
              </div>
              {findingError && <div className="alert">{findingError}</div>}
              <button type="submit" className="btn btn-primary" disabled={busy}>
                Save draft
              </button>
            </form>
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
            <form className="card" onSubmit={onInvite}>
              <h3>Onboard a member</h3>
              <label>
                User email
                <input
                  type="email"
                  value={inviteEmail}
                  onChange={(e) => setInviteEmail(e.target.value)}
                  required
                />
              </label>
              <label>
                Role
                <select value={inviteRole} onChange={(e) => setInviteRole(e.target.value)}>
                  <option value="staff">staff</option>
                  <option value="client">client</option>
                </select>
              </label>
              {inviteError && <div className="alert">{inviteError}</div>}
              <button type="submit" className="btn btn-primary" disabled={busy}>
                Onboard
              </button>
            </form>
          )}
        </aside>
      </div>
    </div>
  );
}
