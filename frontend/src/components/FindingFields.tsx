import { useId } from "react";
import {
  type CreateFindingParams,
  type Finding,
  SEVERITIES,
} from "../api/types";
import { MarkdownField } from "./MarkdownField";
import { TypeInput } from "./TypeInput";

const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

/** The distinct types already used on a project, offered as datalist suggestions. */
export function findingTypes(findings: Finding[]): string[] {
  return [
    ...new Set(findings.map((f) => f.finding_type.trim()).filter(Boolean)),
  ];
}

/** Reads back what `FindingFields` submitted. */
export function findingParams(form: FormData): CreateFindingParams {
  return {
    title: String(form.get("title")),
    finding_type: String(form.get("finding_type")),
    description: String(form.get("description")),
    technical_description: String(form.get("technical_description")),
    impact: String(form.get("impact")),
    recommendation: String(form.get("recommendation")),
    severity: String(form.get("severity")),
  };
}

/** The finding fields, shared by the new-finding form and the edit form. */
export function FindingFields({
  finding,
  types,
}: {
  finding?: Finding;
  types: string[];
}) {
  const typeFieldId = useId();
  return (
    <>
      <label>
        Title
        <input name="title" defaultValue={finding?.title} required />
      </label>
      <label htmlFor={typeFieldId}>
        Type
        <TypeInput
          id={typeFieldId}
          name="finding_type"
          defaultValue={finding?.finding_type}
          suggestions={types}
        />
      </label>
      <label>
        Severity
        <select name="severity" defaultValue={finding?.severity ?? "medium"}>
          {SEVERITIES.map((s) => (
            <option key={s} value={s}>
              {cap(s)}
            </option>
          ))}
        </select>
      </label>
      <div className="field">
        <span className="field-label">Description</span>
        <MarkdownField name="description" defaultValue={finding?.description} />
      </div>
      <div className="field">
        <span className="field-label">Technical description</span>
        <MarkdownField
          name="technical_description"
          defaultValue={finding?.technical_description}
        />
      </div>
      <div className="field">
        <span className="field-label">Impact</span>
        <MarkdownField name="impact" defaultValue={finding?.impact} />
      </div>
      <div className="field">
        <span className="field-label">Recommendation</span>
        <MarkdownField
          name="recommendation"
          defaultValue={finding?.recommendation}
        />
      </div>
    </>
  );
}
