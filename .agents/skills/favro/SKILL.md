---
name: favro
description: Coordinate product, research, business, compliance, operations, and development work in Favro using project-configured roles, workflows, evidence, dependencies, and human handoffs. Use when work should be created, reviewed, tracked, or reconciled in a Favro project.
---

# Favro project coordination

Use Favro as the authoritative coordination surface for work, responsibility, progress, and handoffs. Keep substantive artifacts—code, specifications, research, contracts, designs, and decision/history documents—in their appropriate authoritative systems; cards link to and summarize them.

Use the `favro` CLI from `PATH`. Do not expose credentials in commands, output, cards, comments, or committed configuration.

## Start

1. Run `favro --version` and require version 0.2.1 or newer. A missing version flag indicates a pre-0.2 binary; do not continue with it. Follow [favro-cli/README.md](favro-cli/README.md) to install or replace the CLI in the current agent environment, then verify PATH resolution again.
2. Locate `.favro/project.toml`: explicit `--config`, `FAVRO_PROJECT_CONFIG`, current directory, then parents.
3. Run `favro check` before project mutations. `favro list-collections` is the minimal authentication/connectivity check.
4. If no project is configured, ask the user to choose a name and visibility. Offer the repository/directory name or a generated memorable name. Ask whether interview cards should contain one topic or one question. Then run `favro init`; do not create a collection before the user chooses.
5. If setup or migration is involved, read [references/configuration.md](references/configuration.md).

Project TOML is safe to commit. It contains role/workflow metadata and environment-variable names, never tokens. Credentials remain in environment variables or an ignored env file.

## Work model

- One collection per product or initiative.
- One board per stable role/workstream, not per temporary runtime agent.
- Prefer one role per agent. A role may expand until splitting it improves focus; multi-role coverage is an explicit exception.
- One card has one primary owning role. Other roles contribute on that card; do not create duplicates.
- Use native assignments, dependencies, shared fields, tags, attachments, and lanes whenever supported.
- Cards represent concrete work, questions, decisions, risks, or deliverables—not general notes.

Read [references/workflow.md](references/workflow.md) before designing a new project workflow, conducting interviews, handling human input, completing work, or archiving important history.

## Essential invariants

- New generic workflow: `Inbox → Backlog → Ready → In Progress → Waiting → Blocked → Review → Done`.
- Waiting is an expected pause. Blocked is an unexpected impediment requiring intervention or replanning. Assignment and human-action state are independent of this distinction.
- Every active role has a unique, immutable-for-the-project emoji. Agent comments begin exactly `<emoji> <role>:`. Legacy comments beginning `🤖` remain recognized and editable.
- Anything a human must do goes in the pinned `👤 Needs you` checklist. Assign the configured participant. Expected input moves to Waiting; an unexpected impediment moves to Blocked.
- Done requires satisfied acceptance conditions and a traceable result/evidence in the card, an attachment, or a linked authoritative artifact. Use `favro set-result` before Done.
- Never edit or delete human-authored comments. `comment-edit` and `comment-delete` guard configured agent prefixes and legacy `🤖` comments.
- Archive obsolete/rejected/superseded work to keep live and Done views useful, but record why, any successor, and the configured history reference first.
- External communication, publication, invitations, purchases, subscriptions, or changes outside the selected project require their own authorization.

## Common operations

```bash
favro check
favro add --board product --title "Validate handover need" --template work --type Task
favro move --card <id> --lane "In Progress"
favro set-todo --card <id> --participant owner --item "Answer the handover questions"
favro set-todo --card <id> --participant owner --blocked --item "Choose which conflicting policy controls"
favro set-result --card <id> --text "Interview synthesis: docs/research/handover.md"
favro move --card <id> --lane Review
favro move --card <id> --lane Done
favro review-queue
favro overview
```

Use `--role <role-key>` or `FAVRO_ROLE` whenever a project defines multiple roles; it selects the immutable comment prefix and credential profile. Read [references/cli.md](references/cli.md) for commands, Markdown round-trip constraints, attachments, tags, dependencies, and troubleshooting.

For code-specific review and commit behavior, read [references/development-profile.md](references/development-profile.md). Those rules do not apply to non-code work unless the project enables that profile.
