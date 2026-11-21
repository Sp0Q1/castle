import { Link, useLoaderData } from "react-router-dom";
import { api } from "../api/client";
import type { Person } from "../api/types";

export function loader(): Promise<Person[]> {
  return api.listPeople();
}

function PeoplePage() {
  const people = useLoaderData<Person[]>();

  return (
    <div className="stack">
      <div className="page-head">
        <h1>People</h1>
        <p className="muted">
          Everyone with an account here. A role comes from the identity
          provider; a place on a project is changed on that project.
        </p>
      </div>

      <ul className="list">
        {people.map((person) => (
          <li key={person.id} className="card list-item">
            <div>
              <span className="list-title">{person.name}</span>
              <p className="muted">
                {person.email} · since {person.created_at.slice(0, 10)}
              </p>
              <div className="badges">
                {person.role === "manager" && person.status === "active" ? (
                  <span className="muted">Every project, as a manager</span>
                ) : (
                  person.projects.length === 0 && (
                    <span className="muted">On no project</span>
                  )
                )}
                {person.projects.map((project) => (
                  <Link
                    key={project.id}
                    to={`/projects/${project.id}`}
                    className="person-project"
                  >
                    {project.name}
                    <span className={`badge role-${project.role}`}>
                      {project.role}
                    </span>
                  </Link>
                ))}
              </div>
            </div>
            <div className="person-state">
              {/* Until someone has signed in, the identity provider has not said what they are. */}
              {person.status !== "invited" && (
                <span className={`badge role-${person.role}`}>
                  {person.role}
                </span>
              )}
              <span className={`badge status-${person.status}`}>
                {person.status}
              </span>
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}

export { PeoplePage as Component };
