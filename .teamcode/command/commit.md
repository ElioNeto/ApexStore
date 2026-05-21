---
description: git commit and push

subtask: true
---

commit and push

make sure it follows Conventional Commits with prefixes like:
- feat: (new feature)
- fix: (bug fix)
- refactor: (code change that neither fixes nor adds)
- test: (adding/improving tests)
- docs: (documentation)
- chore: (maintenance, deps, CI)
- perf: (performance improvement)
- revert: (revert a previous change)

For anything related to specific components, include the scope:
- feat(api): ...
- fix(storage): ...
- refactor(cli): ...
- perf(compaction): ...

Prefer to explain WHY something was done from an end user perspective instead of
WHAT was done.

Do not do generic messages like "improved performance" — be very specific
about what user-facing changes were made.

If there are conflicts, DO NOT FIX THEM. Notify the user and they will fix them.

## GIT DIFF

!`git diff`

## GIT DIFF --cached

!`git diff --cached`

## GIT STATUS --short

!`git status --short`
