# Dependency policy

Panoptes commits `Cargo.lock` so development, CI and releases resolve the same
dependency graph. Dependency changes stay small, reviewable and separate from
unrelated work.

## Auditing

`.github/workflows/audit.yml` runs `cargo-audit` on pull requests, pushes to
`main` and daily. The scheduled run detects advisories published after the
last dependency change. GitHub Dependabot alerts and security updates should
also be enabled.

A vulnerability fails CI by default. Triage it within two business days and
prioritise a compatible fixed release; target 48 hours for critical or actively
exploited issues and seven days for other vulnerabilities. Informational
advisories, such as unmaintained crates, require triage but do not fail the
audit.

An audit exception is a last resort, not a way to make CI green. Pass the
advisory ID to `cargo audit --ignore` only with a workflow comment recording
why Panoptes is not affected or how the risk is mitigated, the tracking issue,
an owner and an expiry date. Remove or renew the exception after a fresh review;
never leave an unreviewed permanent exception.

## Routine updates

Dependabot proposes Cargo updates weekly and GitHub Actions updates monthly.
Compatible patch and minor updates are grouped; security updates remain
separate. Review and merge routine updates regularly rather than letting the
queue grow. Production updates are reviewed by a maintainer and are not merged
automatically.

Review major updates individually, including their release notes and breaking
changes. A minor release of a `0.x` dependency can also break compatibility and
needs the same care. Keep a monthly `cargo update` refresh for transitive
dependencies separate from feature work.

`gpui-kit` is pinned exactly because GPUI is pre-1.0 and breaks often. Routine
Dependabot updates ignore it. Upgrade it deliberately, read the upstream
changes and smoke-test the application before merging.

## Accepting a change

Every dependency update must include its `Cargo.lock` change and pass:

```bash
sh scripts/check.sh
```

Updates must preserve the workspace's declared Rust MSRV unless changing that
MSRV is an explicit, documented part of the change. Run
`scripts/smoke.sh` when an update could affect the Pi adapter. GPUI upgrades
and other high-impact runtime updates also need a manual application smoke
test.

For a new direct dependency, first consider whether the existing standard or
workspace libraries already solve the problem. Review its maintenance,
license, enabled features and transitive cost. Use crates.io releases by
default; a git or alternative-registry dependency needs an explicit reason and
review.
