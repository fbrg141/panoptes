// macOS autocorrect rewrites words as they are typed, which mangles prompts,
// paths and identifiers. Every text field turns it off, together with
// auto-capitalisation and browser autofill, through one of these two sets.

/** Prose: the spellcheck underline stays, nothing is replaced. */
export const proseField = {
  spellCheck: true,
  autoCorrect: "off",
  autoCapitalize: "off",
  autoComplete: "off",
} as const;

/** Names, filters and searches: no underline either. */
export const plainField = { ...proseField, spellCheck: false } as const;
