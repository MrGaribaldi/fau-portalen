# FAU project role

Read AGENTS.md and docs/pre-0-guide.md. Coordinate through the existing Favro project
using .agents/skills/favro/SKILL.md.
You develop and operate FAU, not infra-tools upstream or its demo stack.
FAU configuration and code live in /workspace; read-only modules in /opt/infra-tools.
Private Terraform working roots, state, keys and credentials live under
/infra-runtime/infrastructure. Keep them out of /workspace and transcripts.
Follow latest user decisions in docs/planning-decisions.md. Prepare and inspect plans
before infrastructure changes. ALLOW_TERRAFORM_MODIFY does not authorize speculative apply.

## Language

Write all technical content in English: architecture, ADRs, infrastructure, code, decision
documents, Favro cards and agent comments. The user asked for this because Norwegian technical
vocabulary reads badly in translation. Existing Norwegian card text stays unless it is being
edited anyway.

The product itself is a Bokmål application — that is the standard locale, and it must support
Nynorsk and other translations as an architecture requirement, not a later add-on. Never author
Nynorsk content: deliver the localisation mechanism and Bokmål source strings, and leave
translation to a human.

## What FAU is

A Bokmål web app for Norwegian FAU-er (school parents' councils): documents, minutes,
tasks, organisation and audited history that survive yearly turnover. Product premises in
prosjektgrunnlag.md. Stack per docs/repo-container-contract.md: Rust backend, TypeScript
frontend, PostgreSQL, private S3, self-hosted on Hetzner via infra-tools Terraform modules.
Compose project `fau-mvp` currently runs only the `agent` service plus data volumes; app
and db services come later.

## Where the project stands (10 September 2026)

Planning is largely reviewed; application implementation has not started. The workspace went under
git on 10 September, one repo per ADR-001 (`git init` run by Erik on the host; the agent leaves
changes in the working tree). `.gitignore` excludes `.env*` except `.env.example`, Terraform state
and plans, and `favro-cli/target/` - but the favro-cli **source** is committed deliberately, because
`Dockerfile.agent` builds the CLI from it. Details in docs/planning-decisions.md. Stages 0 and 1 are
applied. Stage 0: Cloud SSH key `fau-admin` and the private hel1 buckets
`fau-k3s-backup`/`fau-db-backup`, with `prevent_destroy = true`. Stage 1, applied 9 September:
three `cx23` in hel1 (`master`, `vpn-router`, `cworker-1`), the `lb-fau` load balancer and the
`fau-backend` network, running k3s v1.35.2 with all nodes Ready. Compute cost has started, about
EUR 25.46/month. **Stage 2's CCM pass is applied**, 10 September on Erik's
authorization on #3483: the plan was regenerated first and came out identical to the saved
one, apply added 6 resources, the `uninitialized` taint is gone from all three nodes, coredns,
local-path-provisioner and metrics-server are Running, and a fresh plan reports no drift.
Source is `/workspace/infrastructure/2-cluster`, synced with
`infrastructure/sync-to-runtime.sh 2-cluster`.
Scope is deliberately the `cluster` module only — cert-manager, flux, telemetry and cloudnativepg
each wait on an open decision. Details in docs/stage-2-readiness.md. Terraform state is remote in the private `fau-tfstate`
bucket, holds private keys in clear text, and is versioned with permanent deletion denied by
bucket policy. No copy outside Hetzner, decided 9 September: the keys are regenerable and
account loss would take the infrastructure too. Robot registration is not a trigger; the one
trigger is GitOps secrets encrypted to the SOPS age key, which a Proton Pass export removes.
Details in docs/stage-0-readiness.md; every decision in docs/planning-decisions.md.

Email is settled in direction, not in provider: managed European transactional service,
self-hosted SMTP ruled out, Scaleway TEM Essential provisional for dev/test/pilot, production
pending the verification list on #3410.

Identity and encryption are designed but not built — ADR-003, docs/identity-and-encryption.md,
#3484 in Review. The trust model is the load-bearing part: we hold the keys, so encryption defends
against a leak and not against us, and the phrase "we cannot see your data" is never used. Zitadel
Cloud is a time-boxed exception to the European rule (Swiss subsidiary of a Californian parent);
no European replacement has been found, and the portability rules in decision 3 are binding on the
first line of application code. Authentication is passwordless by email — link or emailed code is
an implementation choice, not an architecture one.

Alerting is decided but not configured, accepted 10 September on #3442: Alertmanager routes warning
to email and critical to Signal via Healthchecks.io (Latvia), with the always-firing Watchdog as the
dead man's switch that catches cluster death. Signal itself is US-owned, permitted as an internal
service carrying scrubbed content — and the boundary covers check names too, since the name is what
reaches the phone. Recorded in docs/internal-supplier-ownership.md. One test remains before wiring:
whether the alert body survives into the Signal message, or the check names must carry the meaning.

Supplier ownership is not a flat "everything European" rule: customer-facing services — anything
touching FAU users or their data — must be European, while FAU's own internal services may be
non-European (Canadian explicitly fine, American AI tooling already in use). An internal tool
that starts handling member data crosses the line and needs a new decision.

VPN stays on WireGuard, decided 9 September on effort and cost, not ownership. Tailscale was
assessed and parked in archived #3440.

Open with the user: #3446, #3439 and #3484 in Review, plus #3481 in Waiting. #3484 is ADR-003.
#3483 stage 2's CCM pass was applied and accepted 10 September and is Done, as is #3482 stage 1.
#3442's alerting endpoint was accepted the same day and the supplier record is written. #3407, #3411 and #3406 were accepted 9 September; follow-ups are #3441 and
#3439. How the agent reaches the private
network is settled: WireGuard works from this container since the 9 September rebuild, so
`kubectl` talks to `https://10.0.1.250:6443` directly. The tunnel dies with the container — raise
it per container with `.agents/skills/fau-vpn/SKILL.md`. The vpn-router SSH bastion is the
fallback.

## Favro: what is not in the skill

The skill covers commands; these facts cost a session to rediscover.

- CLI is on PATH (`favro 0.2.1`), config at `/workspace/.favro/project.toml`, credentials in
  `/infra-runtime/infrastructure/favro.env`. Collection `FAU-plattform`, 12 role boards, one
  emoji per role. Verify with `favro check`. 0.2.1 is baked into the image at
  `/usr/local/bin/favro` as of the 9 September rebuild, so it survives container recreation and
  the 0.2.0 attachment defect is gone. Source is vendored at `.agents/skills/favro/favro-cli`,
  pinned to upstream commit `e0965df` — provenance, the no-hand-editing rule and the re-vendor
  procedure are in `.agents/skills/favro/UPSTREAM.md`. `SKILL.md` and `references/` are upstream's
  files too, so project-specific Favro facts belong in this file, not in them.
- Card IDs: `get`, `comments`, `move`, `set-*` take the 24-hex **cardCommonId**, not `#3438`.
  `favro overview` prints both; it lists live cards only, and there is no archived listing.
- **Agents post with the user's own token, so every comment is attributed to "Erik W.
  Bjønnes".** Author name never tells you who wrote it. Agent comments start with the role
  emoji prefix (`💾 Infrastruktur…:`) or legacy `🤖`; a comment without that prefix is the
  user's own.
- The user's other input channel is the card itself: ticks in the pinned `👤 Needs you`
  checklist (`tasksDone`/`tasksTotal` in `favro get`) and text they append to the description,
  e.g. a "Fra Erik …" block. Diff those against the 🤖 NOTES and RESULT blocks the agent owns.
- **Verify every attachment claim before writing or trusting it.** `favro get` →
  `attachments[].name` is the only truth; card text saying "vedlagt på kortet" has been wrong on
  live cards and cost the user a review. Audit snippet in the skill's references/cli.md.
  Inversely, `tenant-role-history-design-approved.md` (#3412) exists only as a card attachment,
  not in docs/.
- Attachments survive description writes as of 0.2.1, which also fails loudly if it cannot
  preserve them; 0.2.0 destroyed them silently. The caveat that replaces it: whole-description
  writes are read/modify/write, so don't edit one card's description and its attachments
  concurrently.
- `set-notes` and `set-result` leave the `👤 Needs you` ticks and the user's own description text
  intact, but the Markdown round trip collapses consecutive blank lines. `set-todo` rewrites the
  whole checklist and **wipes existing ticks** — read the current state first and only rewrite
  when items actually change.
- **`set-todo` also moves the card to Waiting**, by design: a pinned human action means expected
  input. So set the lane *after* the checklist, never before, or the move is silently undone.
  `set-desc`, `set-notes` and `set-result` do not touch the lane. Verified 10 September 2026.
- `move --lane Done` is refused unless the card passes through Review first; move to Review, then
  Done.
- Never edit or delete the user's comments; record decisions they make on cards into
  docs/planning-decisions.md.
