import { Suspense, lazy, memo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

const MarkdownCodeBlock = lazy(() => import("./MarkdownCodeBlock"));

interface MarkdownBlockProps {
  content: string;
}

export const MarkdownBlock = memo(function MarkdownBlock({ content }: MarkdownBlockProps) {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      components={{
        code({ node, inline, className, children, ...props }: any) {
          const match = /language-(\w+)/.exec(className || "");
          const isInline = inline || !match;
          if (!isInline && match) {
            const codeString = String(children).replace(/\n$/, "");
            return (
              <Suspense
                fallback={
                  <pre className="message-markdown-fallback message-code-fallback">
                    <code {...props}>{codeString}</code>
                  </pre>
                }
              >
                <MarkdownCodeBlock codeString={codeString} language={match[1]} />
              </Suspense>
            );
          }
          return (
            <code 
              className={className} 
              style={{
                backgroundColor: "var(--color-bg-surface-active)",
                padding: "0.2rem 0.4rem",
                borderRadius: "0.25rem",
                fontSize: "0.85em",
                fontFamily: "monospace"
              }}
              {...props}
            >
              {children}
            </code>
          );
        },
      }}
    >
      {content}
    </ReactMarkdown>
  );
});
