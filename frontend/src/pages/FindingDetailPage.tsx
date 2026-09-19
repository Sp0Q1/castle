import {
  type ActionFunctionArgs,
  Form,
  Link,
  type LoaderFunctionArgs,
  redirect,
  useActionData,
  useFetcher,
  useLoaderData,
  useNavigation,
  useSearchParams,
} from "react-router-dom";
import { api, errorMessage } from "../api/client";
import type { Finding, FindingDetail, Member, Project } from "../api/types";
import { useUser } from "../auth/session";
import { Breadcrumbs } from "../components/Breadcrumbs";
import {
  FindingFields,
  findingParams,
  findingTypes,
} from "../components/FindingFields";
import { Markdown } from "../components/Markdown";
import { MarkdownField } from "../components/MarkdownField";

interface Data {
  finding: FindingDetail;
  project: Project | null;
  members: Member[];
  types: string[];
}

export async function loader({
  params,
  request,
}: LoaderFunctionArgs): Promise<Data> {
  const finding = await api.getFinding(Number(params.id));
  const editing = new URL(request.url).searchParams.has("edit");
  const [project, members, siblings] = await Promise.all([
    api.getProject(finding.project_id).catch(() => null),
    // Needed to mirror the backend's edit/publish permission (staff member).
    api.listMembers(finding.project_id).catch((): Member[] => []),
    editing
      ? api.listFindings(finding.project_id).catch((): Finding[] => [])
      : [],
  ]);
  return { finding, project, members, types: findingTypes(siblings) };
}

export async function action({ params, request }: ActionFunctionArgs) {
  const id = Number(params.id);
  const form = await request.formData();
  try {
    switch (form.get("intent")) {
      case "publish":
        await api.publishFinding(id);
        break;
      case "unpublish":
        await api.unpublishFinding(id);
        break;
      case "delete":
        await api.deleteFinding(id);
        return redirect(`/projects/${form.get("project")}`);
      case "comment": {
        const body = String(form.get("body")).trim();
        if (body) {
          await api.addComment(id, body);
        }
        break;
      }
      default:
        await api.updateFinding(id, findingParams(form));
        return redirect(`/findings/${id}`);
    }
  } catch (err) {
    return { error: errorMessage(err, "Failed to save") };
  }
  return null;
}

function Section({ title, body }: { title: string; body: string }) {
  return (
    <div className="finding-section">
      <h3>{title}</h3>
      <Markdown source={body} />
    </div>
  );
}

function FindingDetailPage() {
  const { finding, project, members, types } = useLoaderData<Data>();
  const user = useUser();
  const [searchParams] = useSearchParams();
  const editing = searchParams.has("edit");
  const edit = useActionData<{ error: string }>();
  const saving = useNavigation().state !== "idle";
  const actions = useFetcher<{ error: string }>();
  const composer = useFetcher<{ error: string }>();

  const myMembership = members.find((m) => m.user.pid === user.pid);
  // Same rule the API enforces: author, a "staff" member of the project, or a manager.
  const canModify =
    user.role === "manager" ||
    finding.author_id === user.id ||
    myMembership?.role === "staff";
  const busy = actions.state !== "idle";

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
          <actions.Form method="post" className="actions">
            <input type="hidden" name="project" value={finding.project_id} />
            {canModify && (
              <Link to="?edit" className="btn btn-ghost">
                Edit
              </Link>
            )}
            {canModify && finding.status === "draft" && (
              <button
                type="submit"
                name="intent"
                value="publish"
                className="btn btn-primary"
                disabled={busy}
              >
                Publish to client
              </button>
            )}
            {canModify && finding.status === "published" && (
              <button
                type="submit"
                name="intent"
                value="unpublish"
                className="btn btn-ghost"
                disabled={busy}
              >
                Unpublish
              </button>
            )}
            {canModify && (
              <button
                type="submit"
                name="intent"
                value="delete"
                className="btn btn-danger"
                disabled={busy}
                onClick={(e) => {
                  if (
                    !window.confirm(
                      "Delete this finding and its comments? This cannot be undone.",
                    )
                  ) {
                    e.preventDefault();
                  }
                }}
              >
                Delete
              </button>
            )}
          </actions.Form>
        )}
        {actions.data?.error && (
          <div className="alert">{actions.data.error}</div>
        )}
      </div>

      {editing ? (
        <Form className="card" method="post">
          <h3>Edit finding</h3>
          <FindingFields finding={finding} types={types} />
          {edit?.error && <div className="alert">{edit.error}</div>}
          <div className="actions">
            <button type="submit" className="btn btn-primary" disabled={saving}>
              Save changes
            </button>
            <Link
              to={`/findings/${finding.id}`}
              className="btn btn-ghost"
              replace
            >
              Cancel
            </Link>
          </div>
        </Form>
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

        {/* Keyed on the thread it posts into, so a posted comment empties the composer. */}
        <composer.Form
          className="card"
          method="post"
          key={finding.comments.length}
        >
          <div className="field">
            <span className="field-label">Add a comment</span>
            <MarkdownField
              name="body"
              height={160}
              placeholder="Markdown supported — drag or paste an image"
            />
          </div>
          {composer.data?.error && (
            <div className="alert">{composer.data.error}</div>
          )}
          <button
            type="submit"
            name="intent"
            value="comment"
            className="btn btn-primary"
            disabled={composer.state !== "idle"}
          >
            Post comment
          </button>
        </composer.Form>
      </section>
    </div>
  );
}

export { FindingDetailPage as Component };
