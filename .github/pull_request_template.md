## Use case

<!-- Who is affected, what problem do they have, and what outcome should this
change produce? Link the bug issue, bet, or observation-log entry. Include
measurable acceptance criteria when possible. -->

## Implementation

<!-- Summarize the important decisions and tradeoffs, not every changed file.
Call out alternatives considered and anything intentionally left out. -->

## Verification

<!-- Provide evidence, not only "tested": exact commands, relevant results,
manual steps, and edge cases. Explain why any check below is not applicable. -->

## UI evidence

<!-- For user-visible changes, attach before/after screenshots or a recording.
Cover light and dark appearances when styling changes, and keyboard/focus or
font scaling when interaction or layout changes. Write N/A for backend-only PRs. -->

## Compatibility and risk

<!-- Note effects on existing sessions, worktrees, settings, and stores; rollout
or rollback concerns; and any remaining risk. For SQLite changes, describe the
migration and evidence that an existing store still opens. Write N/A if none. -->

## Definition of done

<!-- A PR is ready to merge only when every applicable item is checked. Mark an
item N/A with a short reason instead of leaving an applicable item unchecked. -->

- [ ] The use case and acceptance criteria are satisfied, and the diff is focused on them.
- [ ] Behavior changes have automated tests, including relevant regression, failure, and boundary cases.
- [ ] `sh scripts/check.sh` passes locally; its result is recorded under Verification.
- [ ] User-facing behavior was exercised manually on macOS, including accessibility and visual states where relevant.
- [ ] Pi adapter, event, or history changes pass `sh scripts/smoke.sh` with a real Pi; otherwise this is N/A.
- [ ] SQLite changes append a migration and test upgrade/reopen with existing data; otherwise this is N/A.
- [ ] Harness-neutral boundaries, the event contract, and design-system tokens remain intact.
- [ ] User docs, developer docs, and shortcuts are updated where behavior or workflows changed.
- [ ] No secrets, prompt/file contents in event logs, debug leftovers, or unrelated generated artifacts are included.
