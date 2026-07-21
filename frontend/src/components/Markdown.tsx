import MDEditor from "@uiw/react-md-editor";
import rehypeSanitize from "rehype-sanitize";
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

/** Renderer overrides: ```mermaid blocks become diagrams; other code is default. */
export const markdownComponents = {
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
