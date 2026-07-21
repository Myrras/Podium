/**
 * Renders a markdown string as sanitized React elements (GitHub-flavoured:
 * tables, task lists, strikethrough, autolinks). react-markdown builds real
 * DOM nodes — it never uses `dangerouslySetInnerHTML` and we add no
 * `rehype-raw`, so embedded HTML is ignored rather than executed. That keeps
 * to-do comments safe to render even though anyone (user or agent) can author
 * them, and works under Podium's locked-down CSP.
 */

import type { AnchorHTMLAttributes } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import { openExternalLink } from "../lib/links";
import styles from "./Markdown.module.css";

// Plain `<a target="_blank">` clicks resolve to a `window.open` new-window
// request the webview has no handler for (see `src/lib/links.ts`), so
// markdown-authored links need the same explicit-open treatment as the
// to-do link chips.
function MarkdownLink({
  href,
  children,
  ...rest
}: AnchorHTMLAttributes<HTMLAnchorElement>) {
  return (
    <a
      {...rest}
      href={href}
      target="_blank"
      rel="noreferrer"
      onClick={(e) => {
        e.preventDefault();
        if (href) openExternalLink(href);
      }}
    >
      {children}
    </a>
  );
}

export function Markdown({ children }: { children: string }) {
  return (
    <div className={styles.prose}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{ a: MarkdownLink }}
      >
        {children}
      </ReactMarkdown>
    </div>
  );
}
