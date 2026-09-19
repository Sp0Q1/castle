import { useEffect, useRef, useState } from "react";
import { errorMessage } from "../api/client";

// Lazy-load mermaid (it is large) and initialize it once, so it lands in a
// separate chunk fetched only when a diagram is actually rendered.
type MermaidApi = typeof import("mermaid")["default"];
let mermaidPromise: Promise<MermaidApi> | null = null;

function getMermaid(): Promise<MermaidApi> {
  if (!mermaidPromise) {
    mermaidPromise = import("mermaid").then(({ default: mermaid }) => {
      // securityLevel MUST stay "strict": diagram source comes from user-authored
      // finding fields and comments. "loose" would let a `click` directive run a
      // javascript: URL (stored XSS) and would stop sanitizing generated markup.
      mermaid.initialize({
        startOnLoad: false,
        theme: "dark",
        securityLevel: "strict",
      });
      return mermaid;
    });
  }
  return mermaidPromise;
}

/**
 * Parses mermaid's SVG output and strips anything executable before it is put
 * into the live DOM: `<script>` elements, `on*` event handlers and
 * `javascript:` links.
 *
 * mermaid's strict mode should already prevent all of these — this is the second
 * layer, so that a mermaid regression (or a future config slip) cannot turn a
 * finding into stored XSS. Returns null when the output is not parseable SVG.
 */
function toSafeSvgNode(svg: string): Node | null {
  const doc = new DOMParser().parseFromString(svg, "image/svg+xml");
  if (doc.getElementsByTagName("parsererror").length > 0) {
    return null;
  }
  const root = doc.documentElement;
  if (root.localName.toLowerCase() !== "svg") {
    return null;
  }
  for (const script of Array.from(root.querySelectorAll("script"))) {
    script.remove();
  }
  const scrub = (el: Element) => {
    for (const attr of Array.from(el.attributes)) {
      const name = attr.name.toLowerCase();
      const value = attr.value.replace(/\s/g, "").toLowerCase();
      const isUrl = name === "href" || name.endsWith(":href");
      if (name.startsWith("on") || (isUrl && value.startsWith("javascript:"))) {
        el.removeAttribute(attr.name);
      }
    }
    for (const child of Array.from(el.children)) {
      scrub(child);
    }
  };
  scrub(root);
  return document.importNode(root, true);
}

// mermaid namespaces the ids and CSS it generates by this, and clears any element
// already carrying it before rendering. The sanitized SVG we mount keeps that id
// (its embedded stylesheet selects on it), so a *stable* id would have mermaid
// delete the diagram currently on screen every time the source changed — which is
// every keystroke in the live preview. A fresh id per render sidesteps it.
let counter = 0;

export function Mermaid({ code }: { code: string }) {
  const host = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    counter += 1;
    const id = `mermaid-${counter}`;
    getMermaid()
      .then((mermaid) => mermaid.render(id, code))
      .then(({ svg }) => {
        if (!active) return;
        const node = toSafeSvgNode(svg);
        if (!node) {
          setError("diagram could not be rendered safely");
          return;
        }
        host.current?.replaceChildren(node);
        setError(null);
      })
      .catch((e: unknown) => {
        if (active) setError(errorMessage(e, "invalid diagram"));
      });
    return () => {
      active = false;
    };
  }, [code]);

  if (error) {
    return <pre className="mermaid-error">{error}</pre>;
  }
  // The sanitized SVG is attached via the ref above — no raw HTML injection.
  return <div className="mermaid-diagram" ref={host} />;
}
