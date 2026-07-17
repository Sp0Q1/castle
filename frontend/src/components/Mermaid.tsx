import { useEffect, useState } from "react";

// Lazy-load mermaid (it is large) and initialize it once, so it lands in a
// separate chunk fetched only when a diagram is actually rendered.
type MermaidApi = (typeof import("mermaid"))["default"];
let mermaidPromise: Promise<MermaidApi> | null = null;

function getMermaid(): Promise<MermaidApi> {
  if (!mermaidPromise) {
    mermaidPromise = import("mermaid").then(({ default: mermaid }) => {
      mermaid.initialize({ startOnLoad: false, theme: "dark", securityLevel: "loose" });
      return mermaid;
    });
  }
  return mermaidPromise;
}

// Unique, valid id for each render (mermaid namespaces internal ids by this).
let counter = 0;

export function Mermaid({ code }: { code: string }) {
  const [svg, setSvg] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    const id = `mermaid-${(counter += 1)}`;
    getMermaid()
      .then((mermaid) => mermaid.render(id, code))
      .then(({ svg }) => {
        if (active) {
          setSvg(svg);
          setError(null);
        }
      })
      .catch((e: unknown) => {
        if (active) setError(e instanceof Error ? e.message : "invalid diagram");
      });
    return () => {
      active = false;
    };
  }, [code]);

  if (error) {
    return <pre className="mermaid-error">{error}</pre>;
  }
  // The SVG is produced by mermaid from our own content, so it is safe to inline.
  // biome-ignore lint/security/noDangerouslySetInnerHtml: trusted, self-generated SVG
  return <div className="mermaid-diagram" dangerouslySetInnerHTML={{ __html: svg }} />;
}
