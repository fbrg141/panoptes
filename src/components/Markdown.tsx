import { openUrl } from "@tauri-apps/plugin-opener";
import { memo, useMemo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { splitMarkdown } from "../markdown";

/**
 * Assistant text as GitHub-flavoured markdown. Raw HTML is not rendered,
 * and links open in the system browser rather than inside the app. The text
 * is parsed chunk by chunk, so a streaming message only re-parses its tail.
 */
export const Markdown = memo(function Markdown({
  text,
  className = "",
}: {
  text: string;
  className?: string;
}) {
  const chunks = useMemo(() => splitMarkdown(text), [text]);
  return (
    <div className={`markdown ${className}`}>
      {chunks.map((chunk, i) => (
        // Chunks only ever grow at the end, so the index is their identity.
        // biome-ignore lint/suspicious/noArrayIndexKey: see above
        <Chunk key={i} text={chunk} />
      ))}
    </div>
  );
});

const Chunk = memo(function Chunk({ text }: { text: string }) {
  return (
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
  );
});
