import { type FormEvent, useEffect, useId, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { api } from "../api/client";
import {
  type FindingDetail,
  type Member,
  type Project,
  SEVERITIES,
} from "../api/types";
import { useAuth } from "../auth/AuthContext";
import { Breadcrumbs } from "../components/Breadcrumbs";
import { Markdown } from "../components/Markdown";
import { MarkdownField } from "../components/MarkdownField";
import { TypeInput } from "../components/TypeInput";

const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

interface Draft {
  title: string;
  finding_type: string;
  description: string;
  technical_description: string;
  impact: string;
  recommendation: string;
  severity: string;
}

function Section({ title, body }: { title: string; body: string }) {
  return (
    <div className="finding-section">
      <h3>{title}</h3>
      <Markdown source={body} />
    </div>
  );
}

export function FindingDetailPage() {
  const { id } = useParams();
  const findingId = Number(id);
  const { user } = useAuth();
  const navigate = useNavigate();

  const [finding, setFinding] = useState<FindingDetail | null>(null);
  const [project, setProject] = useState<Project | null>(null);
  const [members, setMembers] = useState<Member[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [typeSuggestions, setTypeSuggestions] = useState<string[]>([]);
  const [editError, setEditError] = useState<string | null>(null);
  const typeFieldId = useId();

  const [comment, setComment] = useState("");
  const [commentError, setCommentError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = () => {
    setLoading(true);
    setError(null);
    (async () => {
      try {
        const f = await api.getFinding(findingId);
        setFinding(f);
        setProject(await api.getProject(f.project_id).catch(() => null));
        // Needed to mirror the backend's edit/publish permission (staff member).
        const m = await api
          .listMembers(f.project_id)
          .catch(() => [] as Member[]);
        setMembers(m);
      } catch (err) {
        setError(err instanceof Error ? err.message : "Failed to load");
      } finally {
        setLoading(false);
      }
    })();
  };

  useEffect(load, [findingId]);

  const myMembership = members.find((m) => m.user.pid === user?.pid);
  // Same rule the API enforces: author, a "staff" member of the project, or a manager.
  const canModify =
    !!user &&
    !!finding &&
    (user.role === "manager" ||
      finding.author_id === user.id ||
      myMembership?.role === "staff");
  const canPublish = finding?.status === "draft" && canModify;
  const canUnpublish = finding?.status === "published" && canModify;

  const startEdit = async () => {
    if (!finding) return;
    setDraft({
      title: finding.title,
      finding_type: finding.finding_type,
      description: finding.description,
      technical_description: finding.technical_description,
      impact: finding.impact,
      recommendation: finding.recommendation,
      severity: finding.severity,
    });
    setEditError(null);
    setEditing(true);
    const fs = await api.listFindings(finding.project_id).catch(() => []);
    setTypeSuggestions([
      ...new Set(
        fs
          .map((f) => f.finding_type.trim())
          .filter((t): t is string => t.length > 0),
      ),
    ]);
  };

  const setD = (patch: Partial<Draft>) =>
    setDraft((d) => (d ? { ...d, ...patch } : d));

  const saveEdit = async (e: FormEvent) => {
    e.preventDefault();
    if (!draft) return;
    setEditError(null);
    setBusy(true);
    try {
      await api.updateFinding(findingId, draft);
      setEditing(false);
      load();
    } catch (err) {
      setEditError(err instanceof Error ? err.message : "Failed to save");
    } finally {
      setBusy(false);
    }
  };

  const onPublish = async () => {
    setBusy(true);
    try {
      await api.publishFinding(findingId);
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to publish");
    } finally {
      setBusy(false);
    }
  };

  const onUnpublish = async () => {
    setBusy(true);
    try {
      await api.unpublishFinding(findingId);
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to unpublish");
    } finally {
      setBusy(false);
    }
  };

  const onDelete = async () => {
    if (!finding) return;
    if (
      !window.confirm(
        "Delete this finding and its comments? This cannot be undone.",
      )
    ) {
      return;
    }
    setBusy(true);
    try {
      await api.deleteFinding(findingId);
      navigate(`/projects/${finding.project_id}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to delete");
    } finally {
      setBusy(false);
    }
  };

  const onComment = async (e: FormEvent) => {
    e.preventDefault();
    if (!comment.trim()) {
      return;
    }
    setCommentError(null);
    setBusy(true);
    try {
      await api.addComment(findingId, comment);
      setComment("");
      load();
    } catch (err) {
      setCommentError(err instanceof Error ? err.message : "Failed to comment");
    } finally {
      setBusy(false);
    }
  };

  if (loading) return <p className="muted">Loading…</p>;
  if (error) return <div className="alert">{error}</div>;
  if (!finding) return <p className="muted">Not found.</p>;

  return (
    <div className="stack">
      <div className="page-head">
        <Breadcrumbs
          items={[
            { label: "Projects", to: "/" },
            {
              label: project?.name ?? "Project",
              to: `/projects/${finding.project_id}`,
            },
            { label: finding.title },
          ]}
        />
        <h1>{finding.title}</h1>
        <div className="badges">
          {finding.finding_type.trim() && (
            <span className="badge type-badge">{finding.finding_type}</span>
          )}
          <span className={`badge sev-${finding.severity}`}>
            {finding.severity}
          </span>
          <span className={`badge status-${finding.status}`}>
            {finding.status}
          </span>
          <span className="muted">by {finding.author.name}</span>
        </div>
        {!editing && (
          <div className="actions">
            {canModify && (
              <button
                type="button"
                className="btn btn-ghost"
                onClick={startEdit}
                disabled={busy}
              >
                Edit
              </button>
            )}
            {canPublish && (
              <button
                type="button"
                className="btn btn-primary"
                onClick={onPublish}
                disabled={busy}
              >
                Publish to client
              </button>
            )}
            {canUnpublish && (
              <button
                type="button"
                className="btn btn-ghost"
                onClick={onUnpublish}
                disabled={busy}
              >
                Unpublish
              </button>
            )}
            {canModify && (
              <button
                type="button"
                className="btn btn-danger"
                onClick={onDelete}
                disabled={busy}
              >
                Delete
              </button>
            )}
          </div>
        )}
      </div>

      {editing && draft ? (
        <form className="card" onSubmit={saveEdit}>
          <h3>Edit finding</h3>
          <label>
            Title
            <input
              value={draft.title}
              onChange={(e) => setD({ title: e.target.value })}
              required
            />
          </label>
          <label htmlFor={typeFieldId}>
            Type
            <TypeInput
              id={typeFieldId}
              value={draft.finding_type}
              onChange={(v) => setD({ finding_type: v })}
              suggestions={typeSuggestions}
            />
          </label>
          <label>
            Severity
            <select
              value={draft.severity}
              onChange={(e) => setD({ severity: e.target.value })}
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
              value={draft.description}
              onChange={(v) => setD({ description: v })}
            />
          </div>
          <div className="field">
            <span className="field-label">Technical description</span>
            <MarkdownField
              value={draft.technical_description}
              onChange={(v) => setD({ technical_description: v })}
            />
          </div>
          <div className="field">
            <span className="field-label">Impact</span>
            <MarkdownField
              value={draft.impact}
              onChange={(v) => setD({ impact: v })}
            />
          </div>
          <div className="field">
            <span className="field-label">Recommendation</span>
            <MarkdownField
              value={draft.recommendation}
              onChange={(v) => setD({ recommendation: v })}
            />
          </div>
          {editError && <div className="alert">{editError}</div>}
          <div className="actions">
            <button type="submit" className="btn btn-primary" disabled={busy}>
              Save changes
            </button>
            <button
              type="button"
              className="btn btn-ghost"
              onClick={() => setEditing(false)}
              disabled={busy}
            >
              Cancel
            </button>
          </div>
        </form>
      ) : (
        <article className="card">
          <Section title="Description" body={finding.description} />
          <Section
            title="Technical description"
            body={finding.technical_description}
          />
          <Section title="Impact" body={finding.impact} />
          <Section title="Recommendation" body={finding.recommendation} />
        </article>
      )}

      <section className="stack">
        <h2>Discussion ({finding.comments.length})</h2>
        <ul className="list">
          {finding.comments.map((c) => (
            <li key={c.id} className="card comment">
              <div className="comment-head">
                <strong>{c.author.name}</strong>
                <span className={`badge role-${c.author.role}`}>
                  {c.author.role}
                </span>
              </div>
              <Markdown source={c.body} />
            </li>
          ))}
          {finding.comments.length === 0 && (
            <p className="muted">No comments yet.</p>
          )}
        </ul>

        <form className="card" onSubmit={onComment}>
          <div className="field">
            <span className="field-label">Add a comment</span>
            <MarkdownField
              value={comment}
              onChange={setComment}
              height={160}
              placeholder="Markdown supported — drag or paste an image"
            />
          </div>
          {commentError && <div className="alert">{commentError}</div>}
          <button type="submit" className="btn btn-primary" disabled={busy}>
            Post comment
          </button>
        </form>
      </section>
    </div>
  );
}
