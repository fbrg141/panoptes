import { expect, it } from "vitest";
import css from "./index.css?inline";

function layer(source: string, name: string) {
  const match = new RegExp(`@layer\\s+${name}\\s*\\{`).exec(source);
  if (!match)
    throw new Error(
      `Missing ${name} layer in ${source.length} characters: ${source.slice(0, 120)}`,
    );
  const start = match.index + match[0].length;
  let depth = 1;
  for (let i = start; i < source.length; i++) {
    if (source[i] === "{") depth++;
    if (source[i] === "}") depth--;
    if (depth === 0) return source.slice(start, i);
  }
  throw new Error(`Unclosed ${name} layer`);
}

it("places design-system components below Tailwind utilities in the cascade", () => {
  expect(layer(css, "components")).toContain(".px-btn-ghost");
  expect(layer(css, "utilities")).toContain(".bg-surface");
});
