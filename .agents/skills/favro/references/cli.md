# CLI reference and invariants

Run `favro --help` and `favro <command> --help` for the current interface.

## Setup and inspection

```bash
favro init
favro init --name "Product A" --visibility users --interview-granularity topic
favro check
favro list-collections
favro ensure-board --board product
favro overview
favro review-queue
favro list --board product --lane Ready
```

`init` never overwrites an existing TOML. `check`, `list`, `overview`, and `review-queue` are structurally read-only: missing collections, boards, or lanes are reported rather than created. Only `ensure-project` and `ensure-board` create project structure, and should be run after the names and visibility are confirmed. `check` also validates all configured credential variables, role boards, participant/auth user IDs, visibility, and shared fields.

## Cards and metadata

```bash
favro add --board product --title "Validate handover" --template work --type Task
favro add --board backend --title "Reject invalid session" --template code --type Bug
favro move --card <cardCommonId> --lane "In Progress"
favro set-result --card <id> --text "Decision: ...\nArtifact: docs/decision-12.md"
favro priority --card <id> --value High
favro priority --card <id> --value Critical --reason "Production data loss"
favro complexity --card <id> --value 4
favro assign --card <id> --add owner --remove product
favro tag --card <id> --add type:decision
favro depend --card <A> --on <B>
```

Card IDs printed in brackets are `cardCommonId`. The CLI resolves Favro's per-board `cardId` internally. With `--role`, new cards are assigned to that role's configured Favro account. When a Priority field is configured, new cards start at Normal.

## Human actions

```bash
favro set-todo --card <id> --participant owner --item "Answer Q1" --note "Why this matters: ..."
favro set-todo --card <id> --participant owner --blocked --item "Grant access to dataset X"
favro set-todo --card <id> --participant owner --clear
```

The checklist is pinned above other body content. `--clear` removes it. Project configuration is required for automatic assignment/handoff. Setting `compatibility.allow_unassigned_handoffs = true` permits legacy projects to omit an assignee, but the card still moves to Waiting or Blocked.

## Comments and durable state

```bash
favro --role product comment --card <id> --text "Interview synthesis updated."
favro comments --card <id>
favro comments --card <id> --json
favro comment-edit --card <id> --comment <commentId> --text "Corrected text"
favro set-notes --card <id> --text "Current status..."
```

Comments are short chronological signals. Durable current state belongs in maintained body blocks or authoritative linked documents. Never post agent comments with `--raw` if they may need correction. Edit guards accept only the selected role prefix and legacy `🤖`; other roles and all human comments are protected.

## Archive and retarget

```bash
favro archive --card <id> --reason "Superseded" --successor "#123" --history-ref docs/project-history.md
favro archive --card <id> --undo
favro archive --card <id> --undo --instance <cardId> # when several archived board instances exist
favro move-board --card <id> --board market --lane Ready
```

Prefer retargeting when the requirement remains valid. Archive only when its substance is obsolete/rejected/superseded. A retargeted card can have several archived per-board instances; `archive --undo` lists their `cardId` values and requires `--instance` when the choice is ambiguous.

## Attachments

```bash
favro attach --card <id> --file docs/report.md
```

The CLI enforces Favro's 10 MiB attachment limit, infers common MIME types, percent-encodes filenames, and verifies the uploaded attachment.

Version 0.2.1 preserves uploaded files when `set-desc`, `set-notes`, `set-result`,
or `set-todo` writes a description. It reads current attachment metadata, includes
the files in the same Markdown update, then re-reads the card and fails loudly if
any attachment name/count is missing. No local originals or re-upload are needed.
Malformed attachment metadata stops the write before mutation. Archive and
move-board also verify attachments; move-board checks the destination before
archiving the source instances. Whole-description writes are read/modify/write,
so avoid concurrent description/attachment edits to the same card.


## Markdown round trips

Use plain double-quoted `\n`/`\t`; the CLI expands them. Favro returns native checkbox tasks as `☐`/`☑` and blank lines with `↵`; whole-description commands normalize these. Numbered Markdown lists lose their numbers, so use explicit `Q1.`, `Q2.` labels. Avoid nesting.

## Runtime state

IDs are cached under `FAVRO_STATE`, `$XDG_CACHE_HOME/favro/state.json`, `$HOME/.cache/favro/state.json`, or a temporary-directory fallback. Cache data is isolated by organization and collection. Run `ensure-board --refresh` after manual structural changes.

## Compatibility aliases

`favro qa-queue` remains an alias for `favro review-queue` for pre-0.2 integrations. Review queues hide archived cards and skip boards lacking the configured review lane with a warning, so one malformed board does not hide all other review work.

## Troubleshooting

- A read-only API token can inspect Favro but cannot create or move cards. Use a token with write access for mutations.
- Favro limits API traffic to 1000 requests per five-minute window. Prefer scoped commands, avoid polling loops, and retry only after the reported window resets.
- `401`/`403` responses usually mean the selected role has the wrong email/token, lacks collection access, or is using a read-only token. Run `favro list-collections`, then `favro check`.
- When an account belongs to multiple organizations, set its configured organization-ID variable.
- Missing boards or lanes do not get repaired by inspection commands. Confirm the intended structure, then run `ensure-project` or `ensure-board` explicitly.
- Cards whose only instance is on an archived Favro board are deliberately hidden from CLI resolution. Un-archive the board in Favro before rescuing or moving those cards.
