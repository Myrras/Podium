---
name: release
description: Ship a Podium change — make sure the PR has its CHANGELOG `## [Unreleased]` entry, pick the bump label (none/minor/major), run checks, then (after explicit user confirmation) squash-merge; CI assigns the version, tags and publishes the release. Use when the user wants to release/ship/merge a change, cut a patch/minor/major, or "release it".
---

# Release Podium

Every merge to `main` is a release. CI's `release` job (`.github/workflows/ci.yml`)
assigns the version, renames `## [Unreleased]`, bumps the manifests + Cargo.lock,
commits `Release vX.Y.Z` to `main`, tags it, creates the GitHub Release and
dispatches the bundle build that attaches the installers. **Never bump the version
or create a tag by hand.**

**One hard stop: explicit user confirmation before merging** (outward-facing, and
it publishes a release).

## 1. Target PR

Which PR? Usually the one just discussed; if several are open, confirm. Note `N`.

```
gh pr view N --json state,baseRefName,headRefName,labels   # OPEN, base main
```

## 2. CHANGELOG entry (on the PR branch)

The PR must add its entry under `## [Unreleased]` in `CHANGELOG.md` — CI's
`changelog` job fails otherwise. Check with `bash scripts/changelog.sh unreleased`.

- **Functional (user-facing) changes only**; internal refactors, tooling, tests
  and chores still need *an* entry for CI, so describe the user-visible effect
  (if truly none, say so briefly under `### Changed`).
- Keep a Changelog headings, in this order, only those that apply: `Added`,
  `Changed`, `Deprecated`, `Removed`, `Fixed`, `Security`.
- Existing voice: a **bold lead sentence** stating the user-visible effect, then
  detail. Behaviour, not code.
- Don't add a version heading, a date, or `[x.y.z]: <url>` link definitions.

Missing or wrong → fix it on the branch, commit, push. Never add a
`Co-Authored-By` trailer.

## 3. Bump label

Patch by default. Infer from the changes and state your choice (honour a level
the user gave): new feature → `minor`, breaking → `major`.

```
gh pr edit N --add-label minor    # or major; nothing for a patch
```

Remove a wrong one with `--remove-label`. `major` wins over `minor`.

## 4. Checks

`gh pr checks N` must be green. Locally, scope to the change: frontend-only →
`pnpm typecheck` · `pnpm lint` · `pnpm test`; Rust or mixed → `just lint` and
`just test`. Report failures; don't push past them.

## 5. STOP — confirm with the user

Show the PR, its `## [Unreleased]` entry, the label (so the resulting bump), and
that the next step **squash-merges PR #N into `main`, which publishes a release**.
Wait for explicit approval.

## 6. Merge + watch

```
gh pr merge N --squash      # the branch auto-deletes (delete-branch-on-merge)
gh run list --workflow ci.yml --branch main --limit 1   # find the run
gh run watch <run-id>
git fetch --tags && git describe --tags --abbrev=0 origin/main
```

Report the new tag and the release URL (`gh release view vX.Y.Z --json url`).
If the `release` job fails, read its log (`gh run view <run-id> --log-failed`) and
report — don't tag or bump manually.

[Keep a Changelog]: https://keepachangelog.com/en/1.1.0/
