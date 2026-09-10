**Title:** Description writes (`set-desc`, `set-notes`, `set-result`, `set-todo`) silently destroy a card's attachments

## Summary

Any command that writes a card's description removes every attachment already on that card. The
command reports success, exits 0, and prints nothing about the loss. Comments, checklist ticks
and description text are unaffected — only attachments disappear.

Affected commands: `set-desc`, `set-notes`, `set-result`, `set-todo`.
Unaffected in the same test: `comment`, `tag`, `move`.
Not tested: `archive`, `move-board`.

Version: `favro 0.2.0`.

## Why this matters

Attachments are how a card carries its evidence, and `[completion] require_result` workflows
point at them. The result is a card that states a document is attached while the file is gone —
the reader opens the card, finds nothing, and the review round is wasted. It has already
happened on five live cards in one working session.

It also defeats the obvious defence. `attach` verifies its own upload by re-reading the card
(`src/main.rs` around line 2066), and that check passes. The attachment is destroyed later, by
the next description write, so "verify after upload" gives false confidence. Any workflow that
attaches a file and then writes a result block — the natural order — loses the file.

## Reproduction

```bash
favro add --board <board> --title "probe" --template work --type Task
# note the cardCommonId it prints
favro attach --card <id> --file ./some.md
favro get --card <id> | jq '.attachments | length'   # => 1
favro set-result --card <id> --text "probe"          # prints success, exit 0
favro get --card <id> | jq '.attachments | length'   # => 0
```

Identical result substituting `set-notes`, `set-todo` or `set-desc` for `set-result`.

**Expected:** the attachment survives a description write.
**Actual:** `attachments` becomes an empty array, silently.

## Where it comes from

The four description writers each issue `PUT /cards/{cardId}` with a body containing only
`detailedDescription`:

| Command | Call site |
|---|---|
| `set-notes` | `src/main.rs:2138` |
| `set-todo` | `src/main.rs:2247` |
| `set-desc` | `src/main.rs:2309` |
| `set-result` | `src/main.rs:2335` |

Attachments are not markdown links inside the description — they are a first-class array on the
card entity, each element carrying `name`, `fileURL` and `thumbnailURL` (signed Favro S3 URLs).
Upload uses a separate endpoint, `POST /cards/{cardId}/attachment`, in `Api::upload`
(`src/main.rs:670`).

So the loss happens server-side, on the card `PUT`: the update appears to replace the card's
attachment list with nothing when the request does not carry it. That makes this a request-shape
bug in the CLI rather than a description-handling bug.

## Investigate before implementing

Establish what Favro's update-card API needs in order to preserve attachments. The API reference
at favro.com/developer is JavaScript-rendered and does not yield the update-card body parameters
to a plain fetch, so test it directly:

- Does the `PUT` preserve attachments if the existing `attachments` array is echoed back?
- Does it support `addAttachments` / `removeAttachments` style fields?
- Can an attachment be re-added by `fileURL` without re-uploading bytes?

The answer decides which fix below is possible.

## Proposed fix, in order of preference

1. **Preserve in the same request.** Read the card, then include its existing attachments in the
   description `PUT`. One request, no window in which the card has no attachment.
2. **Re-add immediately after.** If the API will not take them on the same call, re-add the
   previously read attachments by URL in a follow-up request. Never leave the card with fewer
   attachments than it started with.
3. **Refuse loudly.** If neither works, abort a description write on a card that has attachments
   unless an explicit flag is passed, printing exactly what would be lost. Failing loudly beats
   silent loss.

Re-uploading bytes from local disk is not acceptable: the CLI often will not have the original
file, and a description write should not depend on it.

## Guard rail, regardless of which fix lands

Every description write should read the attachment names before the `PUT` and verify them after,
failing with a clear message if any are missing. That turns a future regression — including a
change in Favro's own API behaviour — from silent loss into a visible error.

## Acceptance

- [ ] The reproduction above ends in `1`, not `0`, for all four commands.
- [ ] Integration test covering `attach` followed by each of the four description writes,
      asserting the attachment survives. There is no test suite yet, so this is likely the first
      test; a test binary that skips without live credentials is fine, but it must be runnable.
- [ ] Post-write verification guard in place and failing loudly.
- [ ] `archive` and `move-board` checked for the same defect, then confirmed safe or fixed.
- [ ] Version bumped, since consumers check a minimum version at startup.
- [ ] Downstream workaround notes removed once released (see below).

## Current workaround, to retire with the fix

Attach **last**, after every description write on a card, and verify attachments at the end of a
batch of edits rather than immediately after upload. Documented downstream in the consuming
project's `.agents/skills/favro/SKILL.md`, `references/cli.md` and `CLAUDE.md`; those notes
should be removed when this ships so the workaround does not outlive the bug.
