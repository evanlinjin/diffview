# Contributing

## Commits

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/):

```
<type>(<scope>): <summary>

<body: why, and what a reader of the diff wouldn't see>

<footers>
```

- **type**: `feat` (something new for the user), `fix` (a bug), `refactor` (no change in
  behaviour), `perf`, `test`, `docs`, `build`, `ci`, `chore` (the rest: releases, tidying).
- **scope** (optional): the part of dv, e.g. `view`, `pane`, `list`, `gh`, `claude`,
  `rebase`, `merge`, `replace`, `skill`.
- **summary**: imperative, lower case, no full stop, about 50 characters and at most 72:
  `fix(pane): keep the cursor on its comment when the pane re-wraps`.
- **body** (optional): wrapped at 72 columns; why the change, not a list of what changed.
- **breaking changes**: `!` after the type or scope (`feat(keys)!: …`) and a
  `BREAKING CHANGE: …` footer saying what to do instead.
- **one change per commit**: a commit's summary says all of it. Fixes from one review round
  are separate commits when they're separate changes.

Releases are their own commit, `chore(release): vX.Y.Z`, holding only the version bump and
the changelog (see [RELEASING.md](RELEASING.md)).

## Checks

Before committing: `cargo test` and `cargo clippy --all-targets` (no warnings). Snapshot tests
refresh with `UPDATE_SNAPSHOTS=1 cargo test`; look at the snapshot diffs before committing
them.
