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
git on 10 September, one repo per ADR-001 (`git init` run by Erik on the host). **From the
23 September image rebuild the agent may commit locally** — `ALLOW_GIT_WRITE: Yes` in
docker-compose.yml — but it has no path to GitHub: no `GH_TOKEN`, no SSH key, and
`.claude/hooks/no-github-push.js` denies `git push`, `gh` and remote changes. Pushing to
`git@github.com:MrGaribaldi/fau-portalen.git` stays Erik's, from the host. Adding a GitHub
credential to this container is a decision, not a config tweak. `.gitignore` excludes `.env*` except `.env.example`, Terraform state
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
Scope was deliberately the `cluster` module only. What the remaining components actually wait on was
audited 10 September in docs/stage-2-remaining-readiness.md (#3485), and two inherited blockers turned
out stale: cert-manager's Cloudflare token is obsolete by the HTTP-01 certificate decision, not
pending, and `db_memory_request` never belonged to the cloudnativepg operator. **ingress-nginx was applied 10 September**: the `hcloud`
provider is now declared in FAU's root (token from `persistent_outputs`, not upstream's
`bootstrap_outputs`), the CCM adopted `lb-fau`, and nginx answers on 77.42.10.236 / port 443 with
its default certificate. cert-manager is next and no longer waits for #3434: the placeholder
`fau-lab.bim.graphics` resolves to the load balancer, `certificate_email` is
`kontakt@ewb-solutions.as`, the empty Cloudflare token is accepted as harmless clutter, and a
`letsencrypt-staging` ClusterIssuer is proposed before production ACME sees a placeholder. All
stages plan clean with no drift. Two hygiene findings from inspecting the applied cluster are on
#3488 (docs/cluster-hygiene-findings-2026-09-10.md): off-node etcd snapshot retention behaves as ~5
hours rather than the configured 168, while local retention honours it; and there are **two default
StorageClasses** — `hcloud-volumes` from the CSI install and k3s's `local-path` — so every PVC and
stateful workload must set `storageClassName` explicitly, or a Postgres volume can land on
node-local disk. Details in docs/stage-2-readiness.md. Terraform state is remote in the private `fau-tfstate`
bucket, holds private keys in clear text, and is versioned with permanent deletion denied by
bucket policy. No copy outside Hetzner, decided 9 September: the keys are regenerable and
account loss would take the infrastructure too. Robot registration is not a trigger; the one
trigger is GitOps secrets encrypted to the SOPS age key, which a Proton Pass export removes.
Details in docs/stage-0-readiness.md; every decision in docs/planning-decisions.md.

Email is settled in direction, not in provider: managed European transactional service,
self-hosted SMTP ruled out, Scaleway TEM Essential provisional for dev/test/pilot, production
pending the verification list on #3410.

Identity and encryption are designed but not built — ADR-003, docs/identity-and-encryption.md,
#3484 in Waiting on seven questions. The trust model is the load-bearing part: we hold the keys, so
encryption defends against a leak and not against us, and the phrase "we cannot see your data" is
never used. **The provider is Hanko** (Hanko GmbH, Kiel), decided 22 September — the third choice
after Zitadel and PropelAuth, so provider facts live in ADR-003 decision 1 only and the rest of the
ADR says "the provider". PropelAuth was dropped because it offers **no DPA on the Free tier**,
which is disqualifying rather than inconvenient. Hanko is German with EU hosting and a DPA, so the
European rule is **satisfied, not excepted**; its infrastructure subprocessors include AWS, which
is US-owned though the data stays in the EU and no Article 44 transfer arises — accepted 22
September because Hanko documents it. Free to 10,000 MAU, then USD 0.01/MAU. **Hanko has no
organisation model**, so it holds an email address and authentication material and nothing else —
no FAU membership, no names. Every authorization decision hits our own database, per operation, and
the portability rules in decision 3 are binding on the first line of application code; Hanko is
open source and self-hostable, so the exit is real. Authentication is passwordless by email — Hanko
sends a six-digit passcode from EU infrastructure, and passkeys are its primary method (whether
they ship in the MVP is open). Administrative two-factor is a second factor enrolled
(`totp_enabled` / `security_keys_enabled` on Hanko's user object) plus a 30-60 minute
session-freshness gate, enforced server-side by us, never by a provider setting; it is deliberately
not per-action step-up, and the sensitive-endpoint list and exact threshold are still open on
#3414. **Deletion is crypto-shredding**, decided 22 September: each FAU has its own
key-encryption key held only in the key service's datastore — never on the tenant row, never in a
Kubernetes Secret, since etcd is snapshotted — so destroying it makes content unrecoverable in
every database backup too. The key store is replicated against ransomware, and deletion is
*queued* against the replica with a few hours' delay that can be cancelled; a queued or bulk
deletion is a critical alert, not a log line. This binds the first migration: key material must
not land in an application table. The timings, set 22 September: a delete request **freezes** the
FAU at once — billing, invites and writes stop, reads continue so members can export — then
confirmation depends on history, not headcount (an FAU that only ever had one member may self-
delete; any other needs the recovery contact's confirmation, with a 7-day wait that restarts after
a school holiday), and the replica window is **7 days**, long enough to survive a Norwegian Easter.
Closing an FAU account and an individual's Article 17 erasure are different requests with different
clocks; do not conflate them. Only the summer holiday restarts the confirmation clock, and one
operator can cancel. **The backend holds an FAU key once per session, not per operation** — in
memory, refreshed by real activity only, zeroised on logout/idle/lease release, with a concurrency
ceiling as the control; per-operation fetching was rejected because it buries the mass-decryption
signal in routine autosave traffic. Encryption is **server-side**: the React editor never holds a
key. Change notifications carry identifier and revision only, so telling an idle viewer costs no
decryption. MVP search is **filenames, decrypted in-session** — nothing leaves the encryption
boundary to make a feature easier. Retention is **per-membership**: an email is kept while any
active membership exists anywhere, lapsing 3 months after the last one ends; member-elected
retention is designed for (a per-account field in the first migration) but not built. Every FAU
must have a recovery contact, and we hold the seat until a nominated school rep is confirmed.
**End-to-end encryption was considered and rejected** 22 September — continuity across total
turnover is the product and escrow is what makes a system not E2E; it would also remove server-side
JSON validation and malware scanning, trading defence against a bad member for defence against
ourselves. Reasoning is in ADR-003 so it is not re-litigated. **FAU will never be a whistleblowing
or reporting channel** — reports about the school, or about an FAU member, belong with the school.
That is a scope boundary, not a deferred feature; keep it out of the roadmap. **Threat-model
boundary**, 22 September: we defend against mistakes and casual misuse by people with legitimate
access; we do not claim to withstand a determined attacker already inside, and never describe the
product as secure against one. It justifies server-side sanitisation of pasted, imported and
published content; it scopes down adversarial hardening against authenticated members.

Alerting is decided but not configured, accepted 10 September on #3442: Alertmanager routes warning
to email and critical to Signal via Healthchecks.io (Latvia), with the always-firing Watchdog as the
dead man's switch that catches cluster death. Signal itself is US-owned, permitted as an internal
service carrying scrubbed content — and the boundary covers check names too, since the name is what
reaches the phone. Recorded in docs/internal-supplier-ownership.md. One test remains before wiring:
whether the alert body survives into the Signal message, or the check names must carry the meaning.

Supplier ownership is not a flat "everything European" rule: customer-facing services — anything
touching FAU users or their data — must be European, while FAU's own internal services may be
non-European (Canadian explicitly fine, American AI tooling already in use). An internal tool
that starts handling member data crosses the line and needs a new decision. There is currently **no
customer-facing exception**: the PropelAuth one lasted a day and died with the provider. If one is
ever needed, record it the way that one was — the reasoning, the data it sees, and the condition
for revisiting it — and note that the nearest thing to an exception now is a European supplier's
own US-owned subprocessor, which is a different and weaker claim.

VPN stays on WireGuard, decided 9 September on effort and cost, not ownership. Tailscale was
assessed and parked in archived #3440.

**The document layer is decided and accepted, 22 September** (#3490 Done, docs/structured-document-architecture.md,
which replaces its 11 September version): **Tiptap OSS over ProseMirror**, with
`prosemirror-collab`'s central authority — **not Yjs, not any CRDT**, because offline is out of
scope and history must be editable (force-undo, redaction, document purge); a CRDT's tombstones
make that impossible. This supersedes **Lexical** (#3493, #3415 — both Done) and the exclusive-lease
model (#3420). The authority is **part of the Rust backend** — no Hocuspocus, no Node. **The server
never applies a ProseMirror step**: clients apply, rebase and materialise; the server orders,
stores encrypted step batches, broadcasts and authorizes. Publication: the publishing client renders
HTML/PDF, **the server sanitises with an allowlist** (never trusts the client), stores it and serves
it static from a **separate origin**, no JS. Three things bind the migration that creates documents: **document
`type` + `model_version`** so spreadsheets and presentations reuse the substrate later; **tasks and
anything else listed across documents are application rows** with an anchor into the document, since
encrypted bodies cannot be queried; and **per-document keys** (ADR-003 decision 5) so purging one
document is provable. Comment and task anchors are position + version, mapped forward. Paste is an
import path and carries the same risk as upload. Deferred: presence/cursors, offline, track changes,
DOCX, spreadsheets, the agent edit API.

**ADR-002 is accepted** (#3446, Done): UUIDv7 with the creation-time leak accepted, the
unverified-school queue reviewed manually by Erik at about a week's turnaround, and **a malware
scanner added to document ingest** — new scope, tracked on #3447. **Scanning runs in-house**, so no
supplier decision and nothing leaves the cluster, on an **ephemeral `cpx32` worker created per batch
and deleted after** (docs/document-ingest-worker.md): delete rather than stop, since Hetzner bills a
server that exists; batch uploads into windows because a node needs minutes to join; no public IPv4,
since the private network already routes `0.0.0.0/0` via vpn-router. Only the "2" generation of cpx
is in stock in hel1 and `cx33` cannot be created there — so node pools name an ordered list of
acceptable types, never one type. The **leaked S3 credentials were rotated 22 September** (#3489, Done): new pair created in the
Console by Erik and written straight into the files so no value entered a transcript, stages 0 and 1
re-initialised and applied via the full control-plane re-install, and verified by a forced etcd
snapshot uploading with `readyToUse=true`. It exposed an upstream defect worth knowing about before
any future replace of `k3s_master_install`: the module's `backup-k3s-server.sh` runs `set -e` then
`aws s3 cp`, but the AWS CLI on the node has **no credentials** — the module never configures it —
so the destroy-time provisioner fails before `systemctl stop k3s` and the replace deadlocks. Worked
around by writing `/root/.aws/` on master by hand; see the secrets list below and
docs/infra-tools-backup-script-no-credentials-issue.md.

**#3484 (ADR-003) and #3439 (localisation) were accepted 22 September, and #3490 and #3420 are
Done — so nothing now blocks the first migration.** The first schema migration (`0002`, #3416)
carries `account.locale`, `tenant.default_locale` and no key material in any application table.
Optional `document.language`, document `type` + `model_version` and tasks as anchored application
rows bind **the migration that creates documents** (#3419/#3490), not `0002` — #3416's approved
design keeps documents out of scope. Localisation: Nynorsk
plumbing only at launch (no `nn-NO.json`, not even a stub), outer locale path prefix on public
marketing pages with Bokmål unprefixed, cookie + `Accept-Language` inside the app, translator works
from a spreadsheet, and plan for more than two locales (English or Sámi named) — so nothing may
hard-code two or assume Latin collation.

Open with the user: #3481, #3485, #3447, #3488 and #3489 in Waiting. #3485 the next stage 2 pass, #3487 the infra-tools raw-chart issue and #3486 the
favro-cli attachment issue. **#3490 is accepted and Done**, and #3420 is closed with its
requirements moved into that architecture — so #3416, #3419, #3421, #3422, #3491 and #3492 have
what they need. The last gate on the first migration is **#3439**, still in Review: two of its
unanswered questions add locale columns to the schema and a third binds public routing.
#3483 stage 2's CCM pass was applied and accepted 10 September and is Done, as is #3482 stage 1.
#3442's alerting endpoint was accepted the same day and the supplier record is written. #3407, #3411 and #3406 were accepted 9 September; follow-ups are #3441 and
#3439. How the agent reaches the private
network is settled: WireGuard works from this container since the 9 September rebuild, so
`kubectl` talks to `https://10.0.1.250:6443` directly. The tunnel dies with the container — raise
it per container with `.agents/skills/fau-vpn/SKILL.md`. The vpn-router SSH bastion is the
fallback.

## Where secrets hide on the nodes

Never `cat` these; filter or grep with an exclusion. Learned by leaking the S3 keys into a session
transcript on 10 September 2026 while investigating an unrelated systemd warning:

- `/etc/systemd/system/k3s.service.d/s3.conf` on `master` — `AWS_ACCESS_KEY_ID` and
  `AWS_SECRET_ACCESS_KEY` in clear text, written by the k3s-master install.
- `/etc/rancher/k3s/config.yaml` on `master` — `etcd-s3-access-key` and `etcd-s3-secret-key`.
- `/root/.aws/credentials` and `/root/.aws/config` on `master` — written by hand 22 September 2026
  because upstream's `backup-k3s-server.sh` calls `aws s3 cp` while the module never configures the
  CLI (docs/infra-tools-backup-script-no-credentials-issue.md). **Outside Terraform's management**,
  so they must be rewritten on every future S3 rotation until the module is fixed.
- `/infra-runtime/infrastructure/.config/persistent_outputs.json` — every generated key, including
  the SSH private key and the SOPS age secret.
- Terraform state in `fau-tfstate` — private keys in clear text, by design.

A transcript persists in `~/.claude/projects`, so a leak there is a real exposure and the answer is
rotation, not deletion.

## Favro: what is not in the skill

The skill covers commands; these facts cost a session to rediscover.

- CLI is on PATH, config at `/workspace/.favro/project.toml`, credentials in
  `/infra-runtime/infrastructure/favro.env`. Collection `FAU-plattform`, 12 role boards, one
  emoji per role. Verify with `favro check`. The binary is baked into the image at
  `/usr/local/bin/favro`, so it survives container recreation — which also means **the vendored
  source and the running binary can differ until the host rebuilds**. Vendored source is 0.2.2 at
  upstream commit `4beaee2` (re-vendored 23 September 2026); the image built 9 September carries
  0.2.1, so check `favro --version` before relying on anything newer. Provenance, the
  no-hand-editing rule and the re-vendor procedure are in `.agents/skills/favro/UPSTREAM.md`.
  `SKILL.md` and `references/` are upstream's files too, so project-specific Favro facts belong in
  this file, not in them.
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
  The #3412 attachment `tenant-role-history-design-approved.md` is byte-identical to the
  committed docs/tenant-role-history-design.md (checked 23 September 2026).
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
- **A card that asks for a decision must carry the evidence.** Erik reads Favro, not the repo, so
  citing `docs/…` gives him nothing to decide from — attach the document with `favro attach` and
  state the options, the recommendation and the consequence in the comment itself. Said on
  #3485, 10 September 2026. Up to 0.2.1 there was no detach or replace, so re-attaching an edited
  file added a second attachment; 0.2.2 adds `attach --replace` (uploads first, then unlinks the
  old one) and `detach`, both needing `--replace-url`/`--url` when a name is ambiguous. Until the
  rebuilt image is running, keep the old discipline: finish the document before attaching it, and
  use `--name` with a date when a newer version must supersede an older one.
- Never edit or delete the user's comments; record decisions they make on cards into
  docs/planning-decisions.md.
