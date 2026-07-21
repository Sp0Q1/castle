import MDEditor from "@uiw/react-md-editor";
import { type ClipboardEvent, type DragEvent, useState } from "react";
import { api } from "../api/client";
import { markdownComponents, safeRehypePlugins } from "./Markdown";

interface Props {
  value: string;
  onChange: (value: string) => void;
  height?: number;
  placeholder?: string;
}

/**
 * A markdown editor (toolbar + preview toggle) that additionally supports
 * dragging or pasting images: each image is uploaded and its markdown
 * `![](url)` is inserted at the caret.
 */
export function MarkdownField({ value, onChange, height = 240, placeholder }: Props) {
  const [uploading, setUploading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const insertAtCaret = (textarea: HTMLTextAreaElement | null, snippet: string) => {
    if (!textarea) {
      onChange(`${value}${snippet}`);
      return;
    }
    const start = textarea.selectionStart ?? value.length;
    const end = textarea.selectionEnd ?? value.length;
    onChange(value.slice(0, start) + snippet + value.slice(end));
  };

  const uploadImages = async (files: File[], textarea: HTMLTextAreaElement | null) => {
    const images = files.filter((f) => f.type.startsWith("image/"));
    if (images.length === 0) {
      return;
    }
    setUploading(true);
    setError(null);
    try {
      const snippets: string[] = [];
      for (const file of images) {
        const { url } = await api.uploadImage(file);
        snippets.push(`![${file.name || "image"}](${url})`);
      }
      insertAtCaret(textarea, `\n${snippets.join("\n")}\n`);
    } catch (e) {
      setError(e instanceof Error ? e.message : "image upload failed");
    } finally {
      setUploading(false);
    }
  };

  const textareaOf = (el: HTMLElement) => el.querySelector("textarea");

  const onDrop = (e: DragEvent<HTMLDivElement>) => {
    const files = Array.from(e.dataTransfer?.files ?? []);
    if (files.some((f) => f.type.startsWith("image/"))) {
      e.preventDefault();
      void uploadImages(files, textareaOf(e.currentTarget));
    }
  };

  const onPaste = (e: ClipboardEvent<HTMLDivElement>) => {
    const files = Array.from(e.clipboardData?.files ?? []);
    if (files.some((f) => f.type.startsWith("image/"))) {
      e.preventDefault();
      void uploadImages(files, textareaOf(e.currentTarget));
    }
  };

  return (
    <div
      data-color-mode="dark"
      className="md-field"
      onDrop={onDrop}
      onDragOver={(e) => e.preventDefault()}
      onPaste={onPaste}
    >
      <MDEditor
        value={value}
        height={height}
        preview="edit"
        onChange={(v) => onChange(v ?? "")}
        previewOptions={{ components: markdownComponents, rehypePlugins: safeRehypePlugins }}
        textareaProps={{
          placeholder: placeholder ?? "Markdown supported — drag or paste an image to upload",
        }}
      />
      {uploading && <div className="md-hint">Uploading image…</div>}
      {error && <div className="alert">{error}</div>}
    </div>
  );
}
