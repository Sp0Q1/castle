import { useId } from "react";

interface Props {
  value: string;
  onChange: (value: string) => void;
  suggestions: string[];
}

/**
 * A one-line type input backed by a datalist of types already used on the same
 * project, so teams can reuse existing classifications.
 */
export function TypeInput({ value, onChange, suggestions }: Props) {
  const listId = useId();
  return (
    <>
      <input
        list={listId}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder="e.g. SQL Injection"
      />
      <datalist id={listId}>
        {suggestions.map((s) => (
          <option key={s} value={s} />
        ))}
      </datalist>
    </>
  );
}
