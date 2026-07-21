import MDEditor from "@uiw/react-md-editor";
import { useEffect, useState } from "react";
import rehypeSanitize from "rehype-sanitize";
import { api } from "../api/client";
import { Mermaid } from "./Mermaid";

// The preview pipeline enables rehype-raw (raw HTML in markdown becomes real
// elements), and findings/comments are untrusted user content — in a tool whose
// whole job is storing attack payloads. Appending rehype-sanitize runs an
// allowlist AFTER rehype-raw, stripping <script>/<iframe>/on* handlers and
// javascript: URLs. It is appended before rehype-prism-plus, so syntax
// highlighting still applies, and the default schema keeps `language-*` class
// names, so ```mermaid detection below still works.
export const safeRehypePlugins = [rehypeSanitize];

// Recursively reconstruct the raw source text of a code block from its hast
// node — needed because syntax highlighting turns `children` into token spans.
function codeText(
  nodes: Array<{ type?: string; value?: string; children?: unknown[] }> = [],
): string {
  return nodes.reduce((acc, node) => {
    if (node.type === "text" && typeof node.value === "string") {
      return acc + node.value;
    }
    if (Array.isArray(node.children)) {
      return acc + codeText(node.children as typeof nodes);
    }
    return acc;
  }, "");
}

/**
 * True for a same-origin upload path, which is the only kind of image src we
 * fetch ourselves. Anything else (an absolute URL a user typed, a data: URI) is
 * left to the browser, and is constrained by the CSP either way.
 */
function isProtectedUpload(src: string): boolean {
  return src.startsWith("/api/uploads/");
}

/**
 * Renders an uploaded image.
 *
 * Serving uploads is auth-gated, so the browser cannot load them directly — see
 * `api.fetchUpload`. This fetches the bytes with credentials attached and hands
 * the <img> an object URL instead, which the CSP's `img-src blob:` permits.
 */
function UploadImage({ src, alt }: { src?: string; alt?: string }) {
  const [objectUrl, setObjectUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    if (!src || !isProtectedUpload(src)) {
      return;
    }
    let active = true;
    let created: string | null = null;
    setFailed(false);
    api
      .fetchUpload(src)
      .then((blob) => {
        if (!active) {
          return;
        }
        created = URL.createObjectURL(blob);
        setObjectUrl(created);
      })
      .catch(() => {
        if (active) {
          setFailed(true);
        }
      });
    return () => {
      active = false;
      // Object URLs pin the blob in memory until explicitly released.
      if (created) {
        URL.revokeObjectURL(created);
      }
    };
  }, [src]);

  if (!src || !isProtectedUpload(src)) {
    return <img src={src} alt={alt ?? ""} />;
  }
  if (failed) {
    return <span className="md-image-error">{alt || "image unavailable"}</span>;
  }
  if (!objectUrl) {
    return <span className="md-image-loading">{alt || "loading image…"}</span>;
  }
  return <img src={objectUrl} alt={alt ?? ""} />;
}

/** Renderer overrides: ```mermaid blocks become diagrams; other code is default. */
export const markdownComponents = {
  img: UploadImage,
  code({
    className,
    children,
    node,
  }: {
    className?: string;
    children?: React.ReactNode;
    node?: { children?: unknown[] };
  }) {
    if (/language-mermaid/.test(className ?? "")) {
      const source = node
        ? codeText(node.children as never)
        : String(children ?? "");
      return <Mermaid code={source.replace(/\n$/, "")} />;
    }
    return <code className={className}>{children}</code>;
  },
};

/** Renders markdown (GFM + mermaid) for the read views. */
export function Markdown({ source }: { source: string }) {
  return (
    <div data-color-mode="dark" className="md-render">
      <MDEditor.Markdown
        source={source || "_(empty)_"}
        components={markdownComponents}
        rehypePlugins={safeRehypePlugins}
      />
    </div>
  );
}
