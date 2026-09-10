# Product workflow

## Lane semantics

- Inbox: captured but not triaged.
- Backlog: accepted work not yet ready or scheduled.
- Ready: sufficiently clear and unblocked to start.
- In Progress: actively being worked.
- Waiting: a normally expected pause, such as an interview response, scheduled approval, or known dependency.
- Blocked: an unexpected impediment requiring intervention or replanning. It may be waiting on a human, another role, access, or newly discovered work.
- Review: evidence is ready for the configured reviewer/approver.
- Done: acceptance conditions passed and traceable result/evidence is recorded.

Human assignment does not determine Waiting versus Blocked. The expected/unexpected nature of the pause does.

## Cards

Keep the initial body small. The default work template is Purpose/outcome, Next action, and Acceptance. The code template is Change and Acceptance. Ownership and status are native metadata. Add assumptions, risks, sources, dependencies, and decisions only when relevant.

Use namespaced tags so dimensions remain clear: `type:decision`, `domain:gdpr`, `horizon:pilot`, `attention:approval`. Do not duplicate lane state in tags. Use native dependencies for ordering.

## Human handoffs

Every human action belongs in the maintained `👤 Needs you` checklist, one imperative/self-contained checkbox per action. Context belongs below the checklist. Assign the relevant configured participant so the card is visible in their Favro work/notifications.

`set-todo` moves expected input to Waiting; pass `--blocked` for an unexpected impediment. Clearing the checklist removes the human assignment and may restore the selected role assignment. Then deliberately move the card to Ready or In Progress.

## Interviews

Ask until relevant ambiguity is resolved; never impose an arbitrary question limit. Split unrelated domains into separate cards. Configuration chooses one card per topic or per question.

Use checkboxes for questions. Human answers remain untouched in comments as raw chronology. Maintain a flat Answers/Findings section in the body:

```text
☑ Q1. Who performs the handover today?
A1. The treasurer normally performs it; the chairperson covers transitions.

☐ Q2. How often does the handover occur?
```

Do not use nested Markdown. Large transcripts and material research conclusions belong in a linked authoritative document. Add follow-up questions when an answer exposes new ambiguity.

## Decisions and disagreement

One role owns the card. Other roles comment from their disciplines. Record genuine disagreement and the evidence behind it; do not smooth it away. Cross-project decisions should have durable decision/history records linked from Favro.

## Completion

Before Done:

1. Verify acceptance conditions.
2. Record result/evidence with `set-result` or link an authoritative artifact.
3. Pass through Review when configured.
4. Clear stale human actions and assignments.

A result can be a document, research synthesis, decision record, attachment, test output, deployed artifact, or Git commit. Git is one evidence system, not the universal completion mechanism.

## Archival and history

Keep live and Done views useful. Work that is rejected, obsolete, or superseded should record its outcome and then be archived. `Not prioritized` remains in Backlog rather than being archived.

When project history is required, update the configured history/decision document first. Then archive with a reason, optional successor, and `--history-ref`. Archiving is reversible and preserves the Favro thread.

## Offline outbox

If the connectivity check fails, do not attempt uncertain writes repeatedly. Continue analysis locally and create a durable `.favro/outbox/` entry for each intended card, comment, move, result, decision, or dependency. Include a stable local ID, timestamp, intended role/board/card, operation, and payload; never include credentials.

When service returns, reconcile in order:

1. Check whether each operation already happened before replaying it.
2. Apply it through the CLI.
3. Record the resulting Favro card/comment ID in the outbox entry.
4. Move the reconciled entry to `.favro/outbox/synced/` rather than silently deleting it.

This is deliberately a semantic reconciliation process: network failure after a request can be ambiguous, and blindly replaying POST requests can duplicate work.
