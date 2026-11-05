Panoptes is a macOS workspace for running and watching coding agents, built on the Pi agent. The brand is one idea: a loop that never stops, with two eyes that never close. It's dark first on a near-black ground, calm, and dense like an IDE. Colour is kept for the logo, the one action that matters, and agent status.

## Logo

- The mark is the bold loop with two pupils, all in **one ink** on a plain ground. The glint in each pupil is a hole, so the ground shows through. Files are in **Logos**; in code, use `LoopMark`.
- Use `panoptes-mark-*` at 32px wide and up. Below 32px (menu bar, favicons, tabs) use `panoptes-mark-small-*`: a heavier line and no glint.
- Inks: on `brand-pupil` or the dark theme, use `brand-bone` or `brand-red`. On `brand-bone` or the light theme, use `brand-pupil` or `brand-cobalt`. On `brand-red`, use `brand-pupil`. On `brand-cobalt`, use `brand-bone`.
- Keep clear space of half the mark's height on every side. The smallest size is 16px wide.
- The lockup (`panoptes-lockup-*`) sets the mark beside "Panoptes" in Gloock, outlined. Use it in the About window, on the website and in the repository README. In the title bar, use the mark at 20px with the name in `wordmark`.
- Never colour the pupils differently from the loop, outline or rotate the mark, add a shadow or gradient, or redraw the eyes. Never use the mark as a generic "view" icon; that's Lucide `eye`.

## App icons

Eight two-tone icons are offered in Settings → Appearance through `AppIconPicker`. **Pupil on bone is the default.** Keep this order.

| Id | Ink on ground | Contrast | Note |
|---|---|---|---|
| `pupil-on-bone` | `brand-pupil` on `brand-bone` | 14.4 | Default |
| `bone-on-pupil` | `brand-bone` on `brand-pupil` | 14.4 | |
| `pupil-on-red` | `brand-pupil` on `brand-red` | 5.2 | |
| `red-on-pupil` | `brand-red` on `brand-pupil` | 5.2 | |
| `bone-on-cobalt` | `brand-bone` on `brand-cobalt` | 5.6 | |
| `cobalt-on-bone` | `brand-cobalt` on `brand-bone` | 5.6 | |
| `bone-on-red` | `brand-bone` on `brand-red` | 2.8 | Low contrast, a style choice |
| `red-on-bone` | `brand-red` on `brand-bone` | 2.8 | Low contrast, a style choice |

- Each icon is in **AppIcons** as a 1024px PNG and an SVG on the macOS grid: an 824px tile with `radius-icon` corners and a soft shadow, and the mark at 68% of the tile.
- Build the bundled app icon from `panoptes-icon-pupil-on-bone.png`. For a macOS 26 Icon Composer file, use the ground colour as the background layer and `panoptes-mark-<ink>.svg` as the single foreground layer.
- Save the chosen id in settings, then set the Dock icon at launch from the matching PNG.

## The pupil and the loop in the interface

The logo isn't only a badge. Its two parts carry meaning through the whole app.

- **The pupil means alive.** Running status and the Live badge use the pupil, a disc with its glint cut out (`Pupil`), never a plain dot. Nothing else in the interface uses that shape.
- **The mark moves when an agent thinks**, and a stroke runs the loop when one starts (`LoopMark`). See Motion.
- **Watching is the product's verb.** Following an agent live uses the `watch` icon in `focus`, in the agent row, menus and the palette. Stopping uses `watch-off`.
- **Four loop icons** are drawn from the logo's curve: `agent`, `session`, `watch` and `watch-off`. Use them for those meanings only, and Lucide for everything else.
- **Nothing is a dead end.** When the command palette has a query, its last row is always "Ask the agent".

## Colour

Two themes, `dark` first. Both share the five `brand-*` constants, which are for the logo, app icons and marketing only. Never use them as UI fills.

- **Grounds.** `bg` is the window. `surface` holds the sidebar, title bar and inspector. `surface-raised` holds inputs, popovers and the command palette. `surface-sunken` holds terminals, logs and code. Separate panels with a 1px `line`, not with shade alone.
- **Text.** Use `ink` for primary text and icons on every ground, and `ink-muted` for paths, timestamps and helper text. Both hold 4.5:1 on all four grounds in both themes. `ink-faint` is only for placeholders and disabled labels, never for information.
- **Cobalt moves the work forward.** The single primary action on a screen (Run, Approve, Merge) is a `primary` fill with `on-primary` text and `primary-hover` on hover. One per screen.
- **Red is the brand's one loud colour, and it means one thing in the interface:** the Live badge while an agent is acting in your shell (`red` fill, `on-red` text). It is never a button, an error, a link or decoration. Errors use `error`, which is lighter and softer so the two never read as one.
- **Light cobalt does the pointing.** Use `focus` for the focus ring (2px solid, 2px offset), the active tab underline, checked checkboxes (tick in `on-focus`), the watching icon and the selected app icon; `link` for links; and `selection` behind selected rows, menu items and text.
- **Overlays.** Dialogs and the command palette sit on `surface-raised` with `shadow-pop`, over `scrim`.
- **Status.** Every status is a word, a shape and a colour, never colour alone:

| Word | Text and glyph | Fill | Shape |
|---|---|---|---|
| Running | `running` | `running-soft` | The pupil, or the thinking LoopMark |
| Waiting | `waiting` | `waiting-soft` | Hollow ring |
| Failed | `error` | `error-soft` | Alert circle |
| Done | `done` | `done-soft` | Check |

- **Diffs.** Added lines go on `diff-add` and removed lines on `diff-remove`, with `ink` text and a +/− sign.

## Type

- **IBM Plex Sans** (`sans`) for all interface text, **IBM Plex Mono** (`mono`) for anything a machine wrote or will read, and **Gloock** (`display`) only for the wordmark, onboarding and empty-state headlines. Never use Gloock below 20px or in dense UI.
- Body text is `body` (13/20). Use `body-strong` for row names and button labels, `label` for field and tab labels, `heading` for dialog titles and inside panes, and `title` for pane titles.
- `caption` is the only style set in capitals, and only for sidebar group headers ("AGENTS").
- Code uses `code` in terminals and diffs, `code-sm` for inline paths, branches, hashes and shortcuts, and `code-strong` for prompts and exit codes.
- Set counters, durations and line numbers with tabular figures.

## Space, shape and density

- Everything sits on a 4px grid: `space-1` for icon-to-label gaps, `space-2` between related controls, `space-3` for control padding, and `space-4` for pane and dialog padding. Keep chrome tight: `space-6` is for onboarding and empty states only.
- Controls are `control` (28px) tall. Use `control-sm` (24px) in toolbars and `control-lg` (32px) for dialog actions and the palette. Agent rows are 44px.
- **Tight corners, like a code editor.** Use `radius-xs` (2px) for keys and checkboxes, `radius-sm` (4px) for controls, pills, badges and rows, `radius-md` (6px) for panels, terminals, menus and the palette, and `radius-lg` (8px) for dialogs. Nothing in the interface is rounder than 8px. `radius-full` is for dots only, never pills or buttons. App icons use `radius-icon`.
- **Borders, not shadows.** Use a 1px `line` between areas and `line-strong` on control borders. Use `shadow-pop`, a short soft shadow with a 1px `line` edge, only for things that float above the window (menus, popovers, the palette, dialogs). Dialogs split their actions off with a `line` above the footer.

## Iconography

- Use **Lucide** (ISC licence) at `icon-ui` (16px) with a 1.5px stroke in `currentColor`. In React, use lucide-react with `strokeWidth={1.5}`, or the `Icon` component for the approved set. Use 14px in pills and dense rows, and 12px inline with `code-sm`.
- The approved set is in **Icons**: the four loop icons first, then Lucide. Add to it from Lucide. Draw a new loop icon only for a concept that belongs to Panoptes alone, from the same lemniscate at the same stroke.
- No emoji in the interface. The loop mark is never a UI icon, except for Panoptes itself and a working agent (the thinking LoopMark).

## Motion

- Use `duration-fast` for hover and press, `duration-base` for panels, popovers and tab changes, and `ease-out` for anything entering. Nothing bounces.
- **Thinking:** while an agent works, the mark shows it is looking (`LoopMark state="thinking"`, one cycle per `duration-glance`). Pick one motion for the whole app with `motion`:
  - `glance` (default): the pupils look left, then right, and blink.
  - `scan`: a gap runs along the loop, like a sweep.
  - `breathe`: the whole mark swells and settles.
  - `sonar`: rings pulse out from the pupils.
- **Loading:** while an agent starts or connects, the loop draws itself and erases, with no track behind it (`LoopMark state="loading"`, one lap per `duration-loop`). Pick one:
  - `draw-erase` (default): the loop draws itself, then erases.
  - `draw`: one stroke chases the path.
  - `comet`: a stroke with a fading tail.
  - `twin`: two strokes chase each other.
- Use the Lucide `loader-circle` only for small waits inside a control.
- Under `prefers-reduced-motion`, both stop and the mark stays still.

## Voice

- Plain, short, sentence case. Say what happened and what to do next. Address the user as "you"; the agent is "the agent" or its session name. The app never says "I".
- Use exactly these status words: **Running**, **Waiting**, **Failed**, **Done**.
- Buttons are verbs: Run, Approve, Stop, Open diff, Discard changes. No exclamation marks and no emoji.
- Put code, paths and commands in `code-sm`, exactly as typed.
- Show durations as "4m 12s" and past times as "12m ago".
- Examples:
  - "Waiting for your approval to run `pnpm test`."
  - "3 agents running."
  - "Failed: exit 1 in `scripts/build-docs.ts`."
  - "No agents running. Start one with ⌘N."
  - "6 files changed, ready to review."

## Components

`LoopMark` and `AppIconPicker` carry the brand. `Button`, `Kbd`, `Tooltip`, `Menu`, `Dialog` and `CommandPalette` are for actions. `Input`, `Select`, `Checkbox` and `Tabs` are for input and navigation. `StatusPill` and `AgentRow` show agent state, and `Terminal` and `Diff` show what an agent did. `Icon` draws the loop icons and the Lucide set. Every component reads its values from the tokens, so a theme change reaches all of them.

## In code

- `exports/panoptes-tokens.css` declares every token as a CSS variable for both themes, plus a `.text-<style>` class per type style. Set `data-theme` on `<html>`.
- `exports/tokens.dtcg.json` holds the same tokens in the W3C format, for Style Dictionary or Figma Tokens.
- Both are generated from `tokens.json`. Regenerate them after a token change and never edit them by hand. Load the fonts from `fonts/`, or from the `@fontsource` packages for IBM Plex Sans, IBM Plex Mono and Gloock.
