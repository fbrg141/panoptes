# Bets

*Opened 2026-11-07. Status: **open**, to be decided at the review below.*

Which one thing Panoptes will be better at than anything else. This is a
live decision with a review point, not a list of someday ideas. When it is
decided, the outcome goes into [decisions.md](decisions.md) and the chosen
bet gets its own issues; the others are closed or explicitly parked. It is
how #48 gets answered.

## Why a bet at all

A worktree per task, with sessions running inside it, is table stakes.
Conductor, Crystal, Terragon and others already do it. The lane spine in
[domain-model.md](domain-model.md) is therefore built fast and solid, not
polished: it has to be dependable, but nobody will choose Panoptes for it.

What makes Panoptes worth choosing has to come from one bet, pursued
properly. Two half-built bets are worth less than one finished one.

## The review

- **When:** two weeks after the split sidebar (#57) lands and lanes are in
  daily use. If that slips, the review happens on 2026-12-05 anyway, on
  whatever has been used by then.
- **Input:** the observation log below, filled in during those two weeks,
  not memory.
- **Output:** decide whether bet 1 is chosen, with a first slice small enough
  to ship in a week, recorded in [decisions.md](decisions.md). "None yet,
  extend by two weeks" is allowed once, with a reason. Bets 2 to 4 were dropped
  on 2026-11-11 because Panoptes observes Pi rather than orchestrating agents.
- **Until then:** no work on any bet beyond what the spine needs anyway.

## The candidates

### 1. The lane as a reviewable unit of work *(recommended)*

A lane becomes something you can review the way you review a pull request,
but with the story behind it: what changed, which prompt caused each change,
what was tried and abandoned, and which tests ran and how they ended. Evaluate
the agent's work from recorded facts—the tree diff, tool calls and command exit
codes—not from the agent's own account of what it did.

- **Builds on** what already exists: the typed event store, tool calls as one
  entity with their output (#24), approvals (#25) and stable message ids
  (#26). Most of the data is already captured; it is not yet tied to the
  diff.
- **Points here if** during real use you keep scrolling transcripts to find
  why a line changed, or you don't trust a lane's diff enough to merge it
  without re-reading everything.
- **First slice:** for each file in a lane's diff, the prompts and tool calls
  that touched it.
- **Risk:** it is only as good as the attribution of edits to tool calls,
  which gets harder when the agent edits through a shell command.

### 2. Honest multi-agent at the lane level *(dropped 2026-11-11)*

Dropped because Panoptes observes one agent per lane rather than coordinating
multiple agents. Kept here only as a record of the rejected direction.

- **Builds on** lanes, and on several lanes per task.
- **Points here if** you often run the same task twice to compare, or wish
  you had.
- **First slice:** start one prompt in two lanes and show the two diffs side
  by side.
- **Risk:** cost and attention. Most tasks don't need two attempts, and
  comparing them is real work for the person.

### 3. Unattended runs with a deferred approval queue *(dropped 2026-11-11)*

Dropped because deferred approvals and unattended coordination add autonomy and
permission complexity outside the minimal observer direction.

- **Builds on** approvals (#25), the Waiting status, and #47 (permissions and
  security boundaries).
- **Points here if** lanes spend most of their time Waiting on you, or you
  hold back from leaving agents running.
- **First slice:** one screen of every pending approval across lanes, answered
  in order.
- **Risk:** safety. Deferring an approval means the agent keeps going on an
  assumption, which needs the boundaries in #47 first.

### 4. Cross-lane awareness *(dropped 2026-11-11)*

Dropped because coordinating relationships between concurrent lanes belongs to
the orchestration direction that Panoptes is not pursuing.

- **Builds on** lanes and their diffs against the base branch. It brushes
  against the deferred question of the base branch moving underneath a lane.
- **Points here if** you hit merge conflicts between lanes, or avoid running
  lanes in parallel for fear of them.
- **First slice:** a mark on each lane that shares changed files with another
  active lane.
- **Risk:** it matters only with many lanes at once; with two or three, a
  person keeps track in their head.

## Recommendation

Bet 1 is the only remaining candidate. It builds on the typed store that
already exists rather than on new machinery, it helps from the first lane rather
than only when many run at once, and it turns the transcript, which Panoptes
already renders well, into something that answers a reviewer's question. Its
facts come from the recorded tree and command results, not from trusting the
agent's summary. The review can still overturn this; the log below is what
would.

## Observation log

Fill in during the two weeks. One line per moment worth remembering, with the
date and the bet it points to. While bug-only intake is active, record feature
ideas here too instead of opening GitHub issues; they wait for the review above.

| Date | What happened | Points to |
|---|---|---|
| 2026-11-12 | A session in lane A created lane B and did all of B's work itself. B looked empty and nothing tied its diff to the conversation. Attribution of edits, shell commands included, across lanes is prototyped on the `bet1-lane-attribution` branch | 1 |
