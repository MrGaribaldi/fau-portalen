**Title:** `attach` cannot replace or remove an attachment, so a revised document accumulates near-duplicates on the card

**Type:** feature request. Nothing is broken; 0.2.1 does what it promises. This is the missing
half of the attachment story.

## Summary

`favro attach` only ever adds. There is no `--replace`, no `detach`, and no other way to remove an
attachment from a card. Re-uploading a revised file under the same name produces **two
attachments with identical names**, and the card gives a reader no way to tell which is current.
Using `--name` to disambiguate helps the reader but leaves the stale file on the card forever.

Version: `favro 0.2.1`.

## Why it matters

An attachment is how a card carries the evidence for a decision, and evidence gets revised. The
workflow the CLI is built for - `set-result`, then a human decides on the card - depends on the
reader trusting that what is attached is what they should read.

Observed today on a live card. An assessment document was attached so the owner could decide from
the card rather than from a repository he does not read. The document was then extended with two
plan results, so the attached copy was already stale, and the only available move was to upload the
new version under a dated name. The card now carries `stage-2-remaining-readiness.md` and
`stage-2-remaining-readiness-2026-09-10-with-plans.md`, one superseding the other, with nothing in
the metadata to say which. The downstream project has had to write a rule into its own agent
instructions - "finish the document before attaching it, there is no detach" - which is a
workaround for a missing command.

It also interacts badly with the guard `attach` already has. After uploading, `attach` re-reads the
card and checks that *some* attachment carries the filename (`src/main.rs`, the block ending in
"did not appear when card ... was re-read"). A duplicate upload satisfies that check, so the
command reports success precisely when it has created the ambiguity.

## The mechanism already supports removal

This is the useful part: no new API capability is needed, because 0.2.1's preservation fix already
proves the shape of the fix.

`Api::write_description` (`src/main.rs:595`) runs every description write through
`attachments::preserve_in_markdown` (`src/attachments.rs:25`), which re-emits each existing
attachment as `![escaped name](<fileURL>)` ahead of the user's text. Its own comment states the
mechanism: *"Markdown PUT rebuilds Favro's attachment list from image nodes, including non-image
uploads ... neither attachments nor addAttachments body fields work."*

If the card's attachment list is rebuilt from the image nodes in the description, then **omitting a
node removes that attachment** - the inverse of what `preserve_in_markdown` does deliberately on
every write. Removal is therefore a matter of writing the description with one node left out.

Worth confirming with a probe before implementing, since the consequence is destructive and the
comment describes preservation rather than removal: on a scratch card with two attachments, PUT
`/cards/{cardId}` with a description carrying only one of the two image nodes, then re-read and
check whether the other is gone from `attachments` - and whether it is unlinked or actually
deleted.

## What blocks it in the current code

One guard has to learn about intent, or every intentional removal will be reported as data loss:

`attachments::verify` (`src/attachments.rs:57`) compares name→count multisets before and after a
write and fails when any count decreases. `verify_attachments` (`src/main.rs:612`) turns that into
`die`. An intentional removal is indistinguishable from the 0.2.0 defect as far as `verify` is
concerned, so it needs an expected-removals argument rather than a boolean escape hatch - the
guard's value is that it still catches *unintended* loss during an intentional one.

Two details that matter because the code already accounts for them:

1. **Names are not unique.** `names()` counts occurrences precisely because "two uploads may share
   a filename". So a replace must identify the outgoing attachment by `fileURL`, not by name, or it
   will remove an arbitrary one of several.
2. **Unrepresentable attachments already refuse the write.** `preserve_in_markdown` errors when a
   `fileURL` is missing or a name cannot be safely escaped, before mutating anything. A removal path
   inherits that guard, which is correct: better to refuse than to write a description that silently
   drops files.

## Proposed interface

In order of usefulness rather than of implementation cost:

1. **`favro attach --card <id> --file <path> --replace`** - upload the new bytes, then rewrite the
   description omitting the outgoing node. Upload first, so the card is never without the file; the
   brief state where two same-named attachments exist is already tolerated by `names()`. Match the
   outgoing attachment by name, and if several share it, require `--replace-url <fileURL>` rather
   than guessing. Preserve node order, so a replace does not churn the rest of the description.
2. **`favro detach --card <id> --name <filename>`** - the primitive, useful on its own for the stale
   files already sitting on cards. Same guard rules; refuse when the name is ambiguous unless a URL
   is given.
3. Optionally, make plain `attach` **warn** when it is about to create a same-named duplicate, since
   today it reports success. A warning alone would have prevented the case above.

## Acceptance criteria

- `attach --replace` leaves exactly one attachment with that name, carrying the new bytes, and the
  rest of the card's attachments untouched.
- `detach --name` removes exactly one attachment and leaves the description text otherwise
  byte-identical apart from the removed node.
- An unintended loss during either operation still fails loudly: `verify` is given the expected
  removals and continues to catch anything else.
- Both refuse, without mutating the card, when the target is ambiguous or when any attachment on
  the card cannot be represented in Markdown.
- `👤 Needs you` ticks, comments and the user's own description text survive both, as they do for
  `set-*` in 0.2.1.
- The live regression test (`tests/live_attachments.py`) is extended to cover replace and detach,
  including the duplicate-name case, since this behaviour cannot be verified against a mock.

## Not in scope

Deleting the underlying file from Favro's storage if the API only unlinks it from the card. If the
probe shows that omitting a node unlinks rather than deletes, say so in the command's output rather
than implying the bytes are gone.
