# Changesets

Every pull request into `develop` **must** include a changeset. CI enforces this
(`.github/workflows/require-changeset.yml`).

Add one with:

```bash
npx changeset      # or: npm run changeset
```

Pick the bump level for your change:

| Level   | Use for |
|---------|---------|
| `patch` | Backward-compatible bug fixes |
| `minor` | Backward-compatible new features |
| `major` | Breaking / incompatible changes |

## How changesets are used here

In fkst-hosted, changesets drive **only the SemVer version number**. At release
time the highest pending bump is applied to the last released version to compute
the next `vX.Y.Z` tag.

The human-readable **release notes** do **not** come from changesets — they live
in `release-notes/release-note-YYYYMMDD-HHMM.md` (copied from
`.github/release-note-template.md`) and are accumulated into the root
`CHANGELOG.md`. We never run `changeset version`; the pending changesets are
simply deleted by the post-release cleanup.
