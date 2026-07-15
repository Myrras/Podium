/**
 * `tiptap-markdown` ships its `MarkdownStorage` shape but doesn't itself
 * augment `@tiptap/core`'s `Storage` interface with it (unlike most first-
 * party Tiptap extensions) — declare it here so `editor.storage.markdown`
 * is typed everywhere instead of every call site casting through `unknown`.
 */
import type { MarkdownStorage } from "tiptap-markdown";

declare module "@tiptap/core" {
  interface Storage {
    markdown: MarkdownStorage;
  }
}
