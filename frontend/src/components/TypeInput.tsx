import { useId } from "react";

interface Props {
  name: string;
  defaultValue?: string;
  suggestions: string[];
  /** Applied to the real <input> so an external <label htmlFor> can bind to it. */
  id?: string;
}

/**
 * A one-line type input backed by a datalist of types already used on the same
 * project, so teams can reuse existing classifications.
 */
export function TypeInput({ name, defaultValue, suggestions, id }: Props) {
  const listId = useId();
  return (
    <>
      <input
        id={id}
        name={name}
        list={listId}
        defaultValue={defaultValue}
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
