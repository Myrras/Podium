/**
 * Renders a markdown string as sanitized React elements (GitHub-flavoured:
 * tables, task lists, strikethrough, autolinks). react-markdown builds real
 * DOM nodes — it never uses `dangerouslySetInnerHTML` and we add no
 * `rehype-raw`, so embedded HTML is ignored rather than executed. That keeps
 * to-do comments safe to render even though anyone (user or agent) can author
 * them, and works under Podium's locked-down CSP.
 */

import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import styles from "./Markdown.module.css";

export function Markdown({ children }: { children: string }) {
  return (
    <div className={styles.prose}>
      <ReactMarkdown remarkPlugins={[remarkGfm]}>{children}</ReactMarkdown>
    </div>
  );
}
