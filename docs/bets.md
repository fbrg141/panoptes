# Product bet review

*Opened 2026-11-07. Status: **open**, for review on 2026-12-05 (#226).*

Panoptes needs one thing it does exceptionally well, chosen from observed daily
use rather than a list of speculative features. A dependable worktree-per-task
spine is necessary, but it is not by itself a differentiator.

## Review

- **Input:** the observation log below, not memory.
- **Output:** choose a bet and a first slice small enough to ship in a week, or
  extend the review once by two weeks with a reason. Record the outcome in
  [decisions.md](decisions.md).
- **Planning:** accepted work gets GitHub issues and milestones after the
  review. Other candidates are closed or explicitly parked.
- **Until then:** feature ideas stay in the observation log rather than new
  GitHub issues. A small fix for pain recorded during daily use is the only
  exception to bug-only intake.

## Candidate: the lane as a reviewable unit of work

A lane should be reviewable like a pull request, with the story behind it: what
changed, which prompt or tool caused each change, and which evidence is unknown.
The tree and command results are facts; the agent's summary is not.

This direction fits Panoptes' observer role and helps from the first lane. Its
main risk is honest attribution, especially for shell edits and changes made by
background processes. Uncertain attribution must remain explicitly unknown.

The lane-scope review, base readiness and per-turn checkpoint foundations are
already built. Turn scopes, attribution, review progress, feedback and safe
reverts remain candidates under #201, not commitments made by this document.

Multi-agent comparison, unattended deferred approvals and cross-lane
coordination were dropped on 2026-11-11 because they conflict with the decision
to observe Pi rather than orchestrate agents.

## Observation log

Record moments worth remembering with a date and the direction they support.
While bug-only intake is active, record feature ideas here too instead of
opening GitHub issues.

| Date | What happened | Points to |
|---|---|---|
| 2026-11-12 | A session in lane A created lane B and did all of B's work itself. B looked empty and nothing tied its diff to the conversation. Attribution of edits, shell commands included, across lanes is prototyped on the `bet1-lane-attribution` branch | 1 |
| 2026-11-13 | A project whose repository moved can only be cleared away (delete its lanes, remove it, add it again), which loses the lanes' history. Pointing the project at the new path would keep it (#84) | none |
| 2026-11-14 | Worktrees pile up because every lane has its own. Letting a lane run in the main checkout for quick one-off tasks would keep that workflow open without a worktree. Clearing finished lanes is tracked as a fix (#264) | none |
| 2026-11-14 | Reusing worktrees from a pool across lanes was considered and not built: it brings back leftover files, stale branches and checkpoints tied to the wrong lane, and needs an "is it clean?" check the audit already called fragile | none |
| 2026-11-17 | Slash commands are essential in daily use: without them, Pi's prompt templates, skills and extension commands can't be found or run from the composer. Taken as a daily-use fix ahead of the review (#273) | none |
| 2026-11-25 | Picking a model while composing a prompt came up: the composer's model menu needs the mouse or a detour to the session header. Shapes considered: focusing the existing model picker with focus restored to the composer on close (the picker's `return_focus` already does this), or a Panoptes-side `/model` command. Either way the scope is the session's model from the next message, as in Pi — not a second model control that can drift. The first shape was taken on 2026-11-26 (below) | none |
| 2026-11-25 | With projects from different contexts (work, personal) in one Lanes tree, asked for saved view filters over the list: chips like All / work / personal, each filter a set of project ids the user picks, stored as an app preference and applied to the sidebar (and cheaply to ⌘P and new-lane creation). Explicitly not groups (#21, dropped): no nesting, no many-to-many, no per-filter instructions or model defaults — those stay parked. Grouping as folders remains a separate, larger idea for later | none |
| 2026-11-26 | Taken ahead of the review as a daily-use fix (like #273): the model menu opens from the composer with ⌘M, reusing the existing picker and its focus return; the lane filters stay parked for the review | none |
