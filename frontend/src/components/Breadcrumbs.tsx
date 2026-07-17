import { Fragment } from "react";
import { Link } from "react-router-dom";

export interface Crumb {
  label: string;
  /** Omit `to` for the current (last) crumb. */
  to?: string;
}

export function Breadcrumbs({ items }: { items: Crumb[] }) {
  return (
    <nav className="breadcrumbs" aria-label="Breadcrumb">
      {items.map((c, i) => (
        <Fragment key={c.label + (c.to ?? "")}>
          {i > 0 && <span className="crumb-sep">›</span>}
          {c.to ? (
            <Link to={c.to}>{c.label}</Link>
          ) : (
            <span className="crumb-current">{c.label}</span>
          )}
        </Fragment>
      ))}
    </nav>
  );
}
