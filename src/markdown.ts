/**
 * Splits markdown into top-level chunks at blank lines outside code fences.
 * While a message streams only its last chunk changes, so each finished
 * chunk is parsed once instead of on every delta. A blank line followed by an
 * indented line continues the chunk, so list items keep their paragraphs.
 */
export function splitMarkdown(text: string): string[] {
  const chunks: string[] = [];
  let current: string[] = [];
  let fence: string | null = null;
  let blank = false;
  for (const line of text.split("\n")) {
    if (fence === null && line.trim() === "") {
      blank = current.length > 0;
      continue;
    }
    if (blank) {
      if (/^[ \t]/.test(line)) current.push("");
      else {
        chunks.push(current.join("\n"));
        current = [];
      }
      blank = false;
    }
    current.push(line);
    const marker = /^\s*(`{3,}|~{3,})/.exec(line)?.[1];
    if (marker && (fence === null || marker.startsWith(fence))) {
      fence = fence === null ? marker : null;
    }
  }
  if (current.length > 0) chunks.push(current.join("\n"));
  return chunks;
}
