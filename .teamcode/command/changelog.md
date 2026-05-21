---

---

Create `UPCOMING_CHANGELOG.md` from the conventional commits since the last release.

1. Gather commits with `git log --oneline <last-tag>..HEAD`
2. Categorize by type: Features, Bug Fixes, Performance, Documentation, etc.
3. Focus on user-facing changes
4. Write in English, present tense, imperative mood

Rules:
- Group by component: Storage Engine, API Server, CLI, TUI, Build/CI
- Prefer what changed for users over what code changed internally
- Start each bullet with a capital letter
- Skip commits that are entirely internal, CI-only, tests, or refactors
- If no notable changes, write "No notable changes."
- Use `git show <hash>` to inspect actual changes, not just commit messages

$ARGUMENTS
