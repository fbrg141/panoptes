# Product bet review

*Opened 2026-11-07. Proposed track recorded 2026-11-27. Status: **open**; the
formal decision is on 2026-12-05 (#226).*

Panoptes is proposed as the observation and review layer for coding agents.
The harness owns execution; Panoptes owns context, navigation, review and
understanding. The review must confirm, reject or extend this direction from
observed daily use, not from the amount of work already invested.

## Review

- **Input:** the observation log below, not memory.
- **Output:** confirm, reject or extend the proposed product track, choose its
  first slice, record the outcome in [decisions.md](decisions.md), and reconcile
  GitHub milestones and Project statuses. Acceptance does not automatically
  approve every parked issue.
- **Until then:** bug-only issue intake remains in force. #226 is the only Ready
  item; feature issues stay `Parked` even when a milestone groups them as a
  proposed outcome. A small fix for pain recorded during daily use is the only
  exception.
- **Documentation:** current behaviour belongs in
  [domain-model.md](domain-model.md), durable rationale in
  [decisions.md](decisions.md), and proposed direction and evidence in this
  file. Delivery work and its status belong in GitHub.

## Planning state

The user-owned
[Panoptes product track](https://github.com/users/fbrg141/projects/2) is linked
to the [repository's Projects tab](https://github.com/fbrg141/panoptes/projects).
GitHub issues and milestones are authoritative for planned work; the Project’s
**Status** controls commitment. A milestone may therefore group a parked issue
as a proposed outcome before the review. The review gate and the three proposed
product tracks are currently represented as milestones there.

#201 is the reviewable-lane umbrella, not another deliverable. Closed issues
are archived from the active board but remain available in GitHub history. The
Project README records these rules so the tracker does not become a second
documentation system.

## Proposed product track

1. **Reviewable lane — depth.** A user can understand and verify one lane's
   work without reconstructing it from the transcript or a terminal. #205 adds
   turn, range, session and human-edit scopes; #206 adds honest attribution
   between changes and the prompts or tool calls that produced them. Uncertain
   evidence stays explicitly unknown.
2. **Subagent visibility — completeness.** The harness creates, controls and
   stops subagents. Panoptes only shows parent/child hierarchy, live and
   retained transcripts, status, tool calls and usage where available. Child
   sessions are read-only and have no independent Panoptes lifecycle, composer
   or lane ownership.
3. **Project groups and scoped sessions — breadth.** User-initiated sessions may
   eventually work at group, project or lane scope. A group names the
   repositories it covers; each repository keeps its own worktree, diff,
   checkpoint, errors and review identity, with no atomic cross-repository
   claim. Harness-owned subagents remain visible beneath their parent and do
   not become independent work scopes.

The third track would deliberately supersede the earlier rejection of groups
and multi-repo work. It needs a decision at #226 and fresh implementation
issues before work starts. The order is **depth first, completeness second,
breadth third**.

## Boundaries

- Harnesses own prompts, tools, subagents and agent behaviour.
- The user initiates work; Panoptes does not autonomously delegate, coordinate
  agents or spawn subagents of its own.
- Git trees are evidence; conversations and tool calls explain that evidence.
- Every repository boundary stays explicit, especially when a session spans a
  group of projects.
- This is not a promise to “spawn agents at every level”; it is visibility and
  review at every level of work.

Lane-scope review, base readiness and per-turn checkpoints are already built.
Later review progress, feedback, safe reverts and hosting-provider integration
are parked follow-ups under #201 and #266, not part of the first milestone.
Multi-agent comparison, unattended deferred approvals and cross-lane
coordination were dropped on 2026-11-11 because they conflict with the observer
direction.

## Observation log

Record moments worth remembering with a date and the direction they support.
While bug-only intake is active, record new feature ideas here too instead of
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
| 2026-11-27 | Product direction agreed as a proposed track, still subject to #226: reviewable lanes first, harness-owned subagent visibility second, then project groups and user-initiated sessions at group, project or lane scope. The harness keeps execution control; Panoptes keeps observation, context and review | track |
| 2026-11-27 | GitHub planning was reconciled into one public, repository-linked Project with four milestones. Existing feature issues were refreshed but remain `Parked`; Status controls commitment, milestones group proposed outcomes, and closed issues were archived from the active board | planning |
