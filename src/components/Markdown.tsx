import { openUrl } from "@tauri-apps/plugin-opener";
import { memo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

/**
 * Assistant text as GitHub-flavoured markdown. Raw HTML is not rendered,
 * and links open in the system browser rather than inside the app.
 */
export const Markdown = memo(function Markdown({ text }: { text: string }) {
  return (
    <div className="markdown leading-relaxed">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          a: ({ href, children }) => (
            <a
              href={href}
              className="underline"
              onClick={(e) => {
                e.preventDefault();
                if (href) void openUrl(href);
              }}
            >
              {children}
            </a>
          ),
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
});
