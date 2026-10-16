---
name: dv-review
description: Review a PR or branch for the dv diff viewer, writing line-anchored comments to dv's review file, or answer the user's replies to an earlier review there. Use when the user asks for a review "in dv", a "dv review", or to answer/address their dv replies.
---

# Reviewing for dv

dv is a terminal diff viewer. It shows your review inline, next to the lines you comment on,
and reloads it whenever the file changes. The user reads it there, replies to your
comments, and copies the ones they agree with into their GitHub review.

## 1. Find the file and the diff

Run `dv review-info` in the repo (add `-p N` for PR N if the user names one that isn't the
current checkout). It prints:

- `file:` the review file to write (create its directory if needed);
- `base:` the commit dv diffs against. `git diff <base>` is exactly what dv shows: base →
  **working tree** (uncommitted changes included);
- `head:` the current commit;
- `exists:` whether there's already a review (if so, read it first);
- `notes:` your notes on this repo (see below). Read them before you start, if they exist.

## 2. Write a review

Read the diff (`git diff <base>`, plus the surrounding code where you need it), then write
the file as JSON:

```json
{
  "head": "<head from review-info>",
  "created": "<now, ISO 8601 UTC, e.g. 2026-10-16T12:00:00Z>",
  "summary": "Markdown: overall verdict, the main risks, anything cross-cutting.",
  "comments": [
    {
      "path": "src/lib.rs",
      "line": 42,
      "start_line": 40,
      "side": "new",
      "body": "Markdown. Say what's wrong, why it matters, and what to do instead."
    }
  ]
}
```

- `path` is relative to the repo root, as in the diff.
- `line` is the line the comment is on. Add `start_line` for a range (`start_line < line`).
  These are **file line numbers**, not diff positions: on `"side": "new"` (the default) the
  line in the working-tree file; on `"side": "old"` (for a removed line) the line in the base.
  Get them from the diff's `@@` headers or by reading the file. Don't guess.
- Comment on changed lines or close to them (within 3 lines). Those can become GitHub
  comments; lines further away still show in dv but can't.
- One issue per comment. Lead with the severity in bold if it helps triage, e.g.
  `**bug:**`, `**nit:**`, `**question:**`. To propose exact code, use a GitHub suggestion
  block (it replaces the commented lines):

  ````markdown
  ```suggestion
  let x = y.checked_add(1)?;
  ```
  ````

Then run `dv review-check`. It checks every comment against the diff and records the text
of the commented lines in each comment's `code` field, and gives each comment an `id`.
Don't write `code` or `id` yourself. dv uses
it to keep comments on their code as the code changes, and shows a comment as outdated once
its code is gone. Fix each `error:` (the comment can't be placed: wrong path or line), and
consider each `warning:`. Run it again until it says `ok`.

Writing the file replaces any review that was there. If one exists, keep its comments that
still apply rather than dropping the user's replies, and always keep the user's own comments
(`"author": "user"`) as they are, `author` included: they're the user's, not yours.

## 3. Answer the user's replies

The user replies to your comments from dv. Each comment can have a `replies` list:

```json
"replies": [
  { "author": "user", "body": "Isn't this checked by the caller?", "created": "…" },
  { "author": "claude", "body": "…", "created": "…" }
]
```

The user can also leave you comments of their own on lines (`"author": "user"` on the comment
itself; Claude's comments have no `author`): questions about the code, or requests. Treat each
like a reply: it's waiting for you while the user has the last word.

When asked to answer them: read the file, and for every comment whose last word (its last
reply, or the comment itself if it has none) is from `"user"` and isn't `"resolved": true`:

- answer by appending `{ "author": "claude", "body": "…", "created": "<now>" }` to its
  `replies`;
- if the user asks for a change, make it in the code, then reply saying what you changed;
- if the user convinced you the comment is wrong, say so in the reply (don't delete it).

The user can reply in dv while you work, and dv writes the same file. So make small, targeted
edits to it (your Edit tool, not rewriting the whole file from what you read earlier), and if
an edit fails because the file changed, read it again and redo that edit. Keep every other
field, comment and reply exactly as it is, `id`s included. Run `dv review-check` afterwards. It moves comments
whose code moved (`fixed:`) by itself. A comment whose code you changed becomes outdated
(`warning: … outdated`). If it still applies to the new code, re-anchor it: set its `line`
(and `start_line`), delete its `code`, and run `dv review-check` again. If your change
addressed it, leave it outdated and say so in your reply.

## 4. Make the changes

When asked to make the changes your comments ask for: read the file, and for every comment
that isn't `"resolved": true` and that `dv review-check` doesn't report as outdated (the user
dismisses comments they don't want by deleting them, so all that are left are wanted):

- make the change in the code (for a `suggestion` block, apply it; otherwise do what the
  comment says, keeping the change as small as the comment implies);
- append `{ "author": "claude", "body": "Changed: <what you did>", "created": "<now>" }` to
  its `replies`. Don't resolve it: the user resolves once they've checked your change.
- if a comment turns out not to need a change, or you're unsure, reply saying so instead.

Then run `dv review-check`: the comments whose code you changed show as outdated, which is
expected. Finally, summarise what you changed (and what you didn't, and why).

## Your notes on the repo

`notes:` from `dv review-info` is a markdown file that outlives any one review, shared by all
the repo's worktrees (it's inside `.git`). It's how what you learn about the repo carries over
to the next PR, so you don't start from nothing each time. The user can edit it too, from dv.

- **Read it** before every task, and follow it: it overrides your own defaults.
- **Update it** when you learn something that will matter for future reviews of this repo:
  its conventions (error handling, naming, test layout, MSRV, feature flags), the parts of the
  code that are subtle, recurring issues, and the user's review preferences (what they
  dismissed as noise, what they always want flagged, how they reacted to your comments).
- **Keep it short and current**: a few headed lists of one-line points, under ~60 lines.
  Rewrite or drop points that turn out wrong; don't log what happened in a particular PR.
  Create it if it doesn't exist yet (its directory may need creating too).

dv also keeps one Claude Code session per PR and resumes it on every run, so within a PR you
keep the conversation; the notes are for what should carry across PRs.

## Notes

- Never post anything to GitHub yourself; the user decides what goes there from dv.
- The file lives inside `.git`, so it's never committed.
