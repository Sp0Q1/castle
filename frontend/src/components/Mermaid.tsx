import { useEffect, useState } from "react";

// Lazy-load mermaid (it is large) and initialize it once, so it lands in a
// separate chunk fetched only when a diagram is actually rendered.
type MermaidApi = (typeof import("mermaid"))["default"];
let mermaidPromise: Promise<MermaidApi> | null = null;

function getMermaid(): Promise<MermaidApi> {
  if (!mermaidPromise) {
    mermaidPromise = import("mermaid").then(({ default: mermaid }) => {
      // securityLevel MUST stay "strict": diagram source comes from user-authored
      // finding fields and comments. "loose" would let a `click` directive run a
      // javascript: URL (stored XSS) and would stop sanitizing generated markup.
      mermaid.initialize({ startOnLoad: false, theme: "dark", securityLevel: "strict" });
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
  // The diagram source is USER content, so we rely on mermaid's strict mode
  // (set above) to sanitize the markup it generates before we inline it.
  // biome-ignore lint/security/noDangerouslySetInnerHtml: mermaid strict-mode output
  return <div className="mermaid-diagram" dangerouslySetInnerHTML={{ __html: svg }} />;
}
