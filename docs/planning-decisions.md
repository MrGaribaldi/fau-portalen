# FAU-plattform planning decisions

Source: Erik W. Bjønnes's answers in the planning conversation, 7 September 2026. These refine prosjektgrunnlag.md. Unresolved details are not decisions.

## Coordination

- Use Favro, collection name FAU-plattform, visibility restricted to Erik and the required agent account. Other organization members must not have access.
- One interview topic per card. Erik W. Bjønnes receives all questions.
- The project document specifies ten specialist roles and their boards/emojis.

## Delivery and commercial direction

- Target Friday 11 September 2026 for an MVP and full landing page; assess how far implementation gets.
- No pilots signed up. MVP and website precede pilot recruitment.
- Self-service signup creates an FAU immediately using FAU name, school name, and leader email; notify Erik of creation.
- Payment processing follows later. Price: NOK 200 plus VAT per month, billed annually.
- Prefer one or two CPX11 or CPX12 Hetzner nodes. Total budget and feasibility require research; no numeric spending cap has been supplied.
- Product name and domain remain undecided and can be revisited.

## MVP scope

- Required: FAU organizations, member invitations, time-limited roles, school organization configuration, Markdown document creation and editing (section 16 items 1–4).
- Store document change sets from the start to enable future history display and restoration. Initial users do not need history, comparison, or restoration interfaces. Storage representation remains undecided.
- Initial uploads: .txt and .md. Word and PDF support later.
- Broader document import and preservation of originals in S3 are nice to have.
- OCR can wait.
- Meetings, minutes, and controlled publication (items 10–12) are deferred together with high priority.
- Tasks are low priority and deferred. Wiki, search, and export are deferred.
- Audit logging is important; every FAU member may view their FAU's audit log. Infrastructure coverage of application audit requirements remains unverified.
- Infrastructure coverage of backups, recovery, deletion, and other item 18 requirements remains unverified.

## Technology

- Infrastructure repository/directory is infra-tools, not build-infra; the earlier name referred to an older project.
- Erik has not deployed this infrastructure before; he reports it is battle-proven by its author.
- Plan how application code and containers fit infra-tools.
- Backend preference: Rust. Frontend: TypeScript with a lightweight framework suited to the task; framework not selected.
- Initial authentication preference: magic links, no stored passwords. Zitadel or a similar service is a research candidate, not a selected dependency.

## Further answers

- Email verification is required. The leader becomes administrator. A person may register on behalf of another leader, and the FAU may become active once the registrant verifies their own email. Whether that registrant also receives administrator privileges, and when the leader receives privileges, remain open.
- No email delivery provider exists yet. Research Hetzner support first, otherwise another European provider.
- Role expiry removes FAU access. Behavior when another valid role remains needs clarification.
- Documents save automatically. Only one person may edit a document at a time; lock and release behavior remain to be specified.
- Application and landing page initially use Norwegian Bokmål only.
- MVP document access has no restrictions within an FAU: every member can read and edit all documents. Restricted access may be useful later.
- Every email must be verified before its account receives access. The registrant also becomes an administrator; administration is shared with the leader.
- A person retains FAU access while another valid role remains. Role changes over summer are expected to be common.
- Closing the editor releases its exclusive edit lock. Fifteen minutes of inactivity releases the lock. An administrator can force-release it. The precise definition of inactivity remains open.
- New-FAU notifications go to fau@ewb-solutions.as.
- The email provider must have European ownership; EU hosting alone is insufficient.

## Open questions

## Membership and editing follow-up

- An account may belong to multiple FAUs, potentially including a municipal KFAU. Present an organization choice at login and a dropdown for switching afterward. Whether KFAU needs distinct MVP functionality remains open.
- Every member may invite others. Who may assign roles and grant administrator privileges remains open.
- Support the school's registered principal acting as administrator and changing memberships to recover a nonfunctioning FAU. Verification, scope, and MVP timing remain open.
- Administrator roles are time-limited, with a six-month grace period to help establish the next administrators. Exact permissions during this period remain open; this qualifies the general access-expiry rule.
- Editing-lock inactivity means no document changes for fifteen minutes.
- On forced lock release, save the latest changes. Offline/unreachable-editor behavior remains unresolved because client-only changes cannot necessarily be retrieved.
- Automate school-year transitions as much as possible later; automation is outside the MVP.

## Latest clarifications

- Only administrators may assign roles and grant administrator access. All members may invite.
- During the six-month administrator grace period, outgoing administrators retain only access needed for handover. Broader access requires a new role. Exact handover operations remain to be defined.
- Principal-led recovery is outside the MVP. Require school name, municipality name, and a link to the municipality's page for the school, intended to identify the principal. Principal verification procedures are not yet defined.
- A KFAU is a separate organization whose members happen to belong to other FAUs; no constituent-FAU integration was requested.
- Forced lock release uses the latest server-saved version and discards any remaining unsaved changes. This supersedes the earlier request to save pending client changes on forced release. A revoked editor must not subsequently overwrite the document with stale changes.
- Manual school-year transition requirements remain unanswered.

## MVP boundary and decision flexibility

- Product decisions may evolve. Traceability, GDPR, and European ownership are firm requirements.
- Administrator handover access initially covers inviting replacements, assigning their roles, and granting administrator access; revisit as needed.
- A member may invite their own replacement or substitute (vikar) and specify this in the invitation. Whether this automatically grants the corresponding role, its dates, and approval requirements remain open.
- Other invitations require administrator role assignment. Read-only membership could be granted initially when that capability exists; it is not selected for the current unrestricted-access MVP.
- KFAU registration details are deferred. Municipal purchasing may introduce additional requirements.
- The proposed manual school-year transition operations can all wait: creating school years, moving cohorts between grades, splitting/merging groups, and changing role dates. How to reconcile this with the required initial organization configuration and time-limited roles needs clarification.

## Confirmed simplifications

These answers supersede the earlier invitation rules and resolve the initial-setup question.

- MVP: only administrators may invite members. Member-initiated invitations, replacement invitations, and vikar invitations are deferred.
- Future vikar behavior: a second person shadows the member throughout the school year and can step in when needed.
- Future replacement behavior: the replacement overlaps with the outgoing member until the next school year starts.
- Initial school organization configuration and time-limited role setup remain required for MVP. Later school-year transitions and organizational changes can wait.

- How the leader email relates to signup verification and initial administrator privileges.
- Email provider and destination/channel for signup notifications.
- Initial role permissions, membership expiry behavior, and school-year transitions.
- Document edit/save semantics, concurrent edits, and retained change-set format.
- Landing page content, language, and launch identity.
- Infrastructure sizing, cost, availability, and operational coverage.

## Infrastructure topology correction

- Erik explicitly directs following the existing infra-tools setup with three servers. This supersedes the earlier preference for one/two CPX11/CPX12 nodes. Verify and cost the existing setup; do not assume a reduced topology or remove existing components.

## Direct S3 uploads

- Agreed: browser uploads directly to private S3 using a short-lived signed URL after backend authorization. Backend creates the pending metadata record and generates the exact object key; client cannot choose bucket, FAU prefix or an existing file key.
- Backend independently checks stored size and validates content before availability. No S3 credentials reach the browser. Downloads continue through the authorization layer.
- Required design/test work: Hetzner-supported upload limits, replay/overwrite protection, safe finalization and abandoned-upload cleanup. Signed URLs are not assumed single-use.
- Shared versus per-FAU bucket topology remains open. See docs/storage-security-proposal.md and Favro #3435.

## Session-based document versions — 7 September 2026

Erik confirmed while reviewing #3412 that each historical document version should be a full snapshot, but changes within an editing session should be combined rather than storing a full version for each autosave/character. This supersedes the initial #3412 proposal of a full snapshot for each saved change.

Erik suggested an optional “Lagre ny versjon” action to split versions while continuing the same session. Its MVP inclusion remains a proposal. The architecture draft distinguishes durable autosaved working content and technical concurrency revisions from finalized historical versions. Timeout/close/forced-release finalization rules are implementation proposals, not additional confirmed interview answers. See the updated attachment on #3412.

## MVP baseline review accepted — 7 September 2026

Erik approved #3405 in Favro at 15:08:37.993 UTC: “Gjennomgangen ser bra ut for nå.” The review checklist is checked. Approval was reconciled after Erik pointed it out in chat. This accepts the MVP baseline as reviewed at that time; subsequent session-based versioning clarification is recorded separately above. It does not approve every architecture proposal on #3412 or authorize deployment.

## Architecture review accepted — 7 September 2026

Erik approved #3412 in chat: “ellers synes jeg #3412 er grei”, after confirming full snapshots grouped by editing session. The reviewed tenant/role/history design is accepted as the implementation basis. This completes the design review, not implementation or deployment. Email address changes preserving account identity are tracked separately in #3437. Explicitly deferred functionality and decisions assigned to other cards remain open/deferred.

## Existing dedicated server candidate — 7 September 2026

Erik asks to evaluate using his idle Hetzner server (reported Intel Core i7-6700, 64 GB RAM, two 512 GB NVMe SSDs) to host the two application/cluster nodes initially, saving costs until market demand is established, then moving to other scalable nodes. This authorizes evaluating consolidation and supersedes the earlier instruction not to investigate alternatives to the three-cloud-server topology. It does not yet select a hypervisor, storage design or final topology, approve downtime, or authorize reinstalling the server. Existing topology also includes a VPN router in addition to master, worker and load balancer; distinguish these in the comparison.

Code review found a dedicated-worker module but no Rescue/installimage or VM-provisioning automation in the inspected checkout. See docs/existing-server-assessment.md and #3408. The application container contract remains applicable across these hosting choices. #3411 remains under review; “flott” followed by this topology question is not recorded as unconditional approval of all ADR proposals.

Erik subsequently explicitly accepts the shared physical-host failure mode for the current phase without paying customers and prioritizes reusing existing infra-tools mechanisms. Do not request this risk acceptance again. He reports the author supports provisioning a Hetzner GPU worker from root SSH access and IP. The local README confirms dedicated CPU/GPU worker bootstrap; this mechanism applies to an ordinary worker as well. It does not itself establish two-VM creation or control-plane relocation. Follow up on the smallest reusable topology, without requiring a hypervisor solely to preserve the earlier logical node count.

Erik reports the current OS is openSUSE Leap and is willing to reset/reinstall it to match infra-tools. For the current MVP test, backup has lower priority; avoiding a difficult future migration is a firm requirement. Do not make a full production backup/restore exercise a prerequisite for starting this test. Preserve a portable PostgreSQL data model, versioned migrations, configurable storage and reproducible deployment. Keep #3425 for later recovery verification. No reset has been executed; server-specific installation details remain to be prepared. Local cloud provisioning selects Ubuntu 24.04; dedicated bootstrap assumes an installed Netplan-capable OS and does not itself install one. Proposed installation path is Hetzner Rescue/installimage followed by existing worker bootstrap.

Stage-0 correction: Erik correctly points out that Terraform generates the required SSH keys. Confirmed in 0-persistent/main.tf: RSA 4096, Cloud registration and local key files. Do not ask Erik to create a separate key. Robot registration is a later public-key handoff. Stage-0 readiness and missing inputs are documented in docs/stage-0-readiness.md; no Terraform binary or stage-0 credentials/state were found in this environment.

## Pre-0 and agent-box — 7 September 2026

Erik confirms S3 should be set up normally; only extensive SSD backup work is deprioritized for the test. He requests a step-by-step Favro checklist for preparing an agent environment outside the current container, so agents can execute the work there. This is tracked as #3438.

After Erik requested review of agent-box, its repository was fetched at 1816d1e6b7ac1c682c294ac5ee3558107c3d15f4 (v1.4.2). Recommended pre-0 now uses a separate FAU workspace/Compose with agent-box and later sibling app/db services. It supersedes the draft that placed FAU inside infra-tools. Shared Terraform modules are reused unchanged via read-only local mount or pinned Git source; FAU owns root configuration and state. Demo-app is a Django runtime and need not host Terraform. Guide/template: docs/pre-0-guide.md and pre-0-template/. No container or infrastructure was started. Project credentials/state live in a private runtime volume outside the code workspace, following agent-box's operating manual. Agent-image rebuilds require a host-side handoff; ordinary app/service work can be driven by the agent.

## Stage-0 naming and location — 8 September 2026

Erik chose, while S10 was prepared: Hetzner location `hel1`, cluster name `fau` (the Hetzner
project is called FAU-portal, but the short name keeps resource names simple), and an explicit
0600 permission on `persistent_outputs.json` in FAU's own root configuration. This settles the
open naming/location item in docs/stage-0-readiness.md and supersedes its `fau-mvp` proposal.
Buckets become `fau-k3s-backup` and `fau-db-backup`; the Cloud SSH key is `fau-admin`.

## Stage 0 applied — 8 September 2026

Erik authorized `terraform apply` in chat after reviewing the concrete plan. Stage 0 ran
successfully: 12 added, 0 changed, 0 destroyed. Created in Hetzner: Cloud SSH key `fau-admin`
(id 118512502) and the private hel1 buckets `fau-k3s-backup` and `fau-db-backup`. The admin
RSA 4096 keypair, three WireGuard keypairs and the SOPS age key were generated locally into
the private runtime volume. `/workspace/.sops.yaml` holds the age public key only.

This is the first real infrastructure FAU owns. Terraform state now lives only in the
`infra-runtime` Docker volume and contains every private key in clear text; it has no backup,
and `docker compose down -v` would destroy it. State backup is an open item. Stage 0 created
no VM, dedicated server, load balancer or OS, so nothing is running yet and no compute cost
has started. The same generated public key is what must later be registered in Hetzner Robot.

## Bucket protection and plan retention — 8 September 2026

Erik chose the safe setting for the stage-0 backup buckets: `prevent_destroy = true` and
`force_destroy = false` on both `fau-k3s-backup` and `fau-db-backup`, diverging from upstream
infra-tools, which ships `force_destroy = true` with the lifecycle block commented out. Applied
as an in-place change; verified that `terraform plan -destroy` now refuses both buckets.

`stage0.tfplan` is kept as an audit trail rather than deleted. It holds credential values and
stays inside the private runtime volume.

Terraform state backup remains open. Nothing consumes the generated SSH, WireGuard or age keys
yet, so losing them today would cost only a re-run of stage 0. That changes once the public key
is registered in Robot or GitOps secrets are encrypted to the age key — a backup must exist
before stage 1.

## Managed European transactional email — 8 September 2026

Erik set the direction for #3410 in the card description at 16:03:46.041 UTC, after a
discussion with ChatGPT, and noted it in a card comment. Self-hosted SMTP is ruled out. The
platform must use a managed, European-owned transactional email service, because the goal is a
SaaS that operates itself as far as possible. This supersedes the open comparison between
Hetzner SMTP and managed services that #3410 originally asked for.

The provider owns SMTP infrastructure, sender IPs and reputation, delivery queues, retries,
bounces, complaints and delivery logs. The application owns only: email templates and
recipients; a PostgreSQL outbox so messages are neither lost nor sent twice; delivery status
and webhook handling; alerting on permanent failures or a stalled queue; and a
provider-independent integration so the service can be replaced later. The integration uses the
provider API primarily, with SMTP as fallback or for components that cannot use the API.

Provisional choice: Scaleway Transactional Email (TEM) Essential for development, test and
pilot. Rationale: managed, both API and SMTP, consumption-based pricing with no monthly fee,
300 emails included and EUR 0.25 per additional 1 000, European-owned (iliad) with EU
processing by default, and a Scale plan with a published 99.9 % API/SMTP SLA available later.
At a conservative 20 emails per FAU per month, 100 FAU-er is roughly 2 000 emails, about
EUR 0.43 per month excluding VAT — negligible at expected volume. Email is sent from a
dedicated subdomain, for example notify.<chosen-domain>, so sender reputation is isolated from
ordinary organisation mail; the domain itself is still open in #3434. fau@ewb-solutions.as
remains the test recipient and the target for operational alerts.

This is a provisional choice, not production approval. Before production the following must be
investigated or confirmed in writing: which subprocessors TEM actually uses in fr-par; whether
"all European" means European provider, contract and EU processing, or that every subprocessor
must also be European-owned, since Scaleway's public list includes some US-owned data centre
operators processing in the EU; retention of message content, recipient addresses, delivery
events, logs and backups; whether anyone outside the EU/EEA can access data; Scale plan pricing
and contract terms; whether the SLA can be delivered without a dedicated IP, as expected volume
is probably too low to build reputation on one; what the SLA actually covers, given that the
published SLA is API/SMTP availability and not inbox placement; and whether a better European
alternative exists with both pure consumption pricing and a production SLA.

Decision path: Essential for development and test; Essential may continue into pilot/early
production if the tests and privacy clarifications are approved; ordinary production is a new
decision based on SLA, data processing agreement, subprocessors, retention and actual traffic.
If Scaleway does not meet the requirements, another managed European service is chosen —
self-hosting on Hetzner is not an alternative.

Required before approval: SPF, DKIM and DMARC on a separate test subdomain; delivery tests to
Gmail, Outlook and at least one Norwegian or European mail provider; tests of invalid recipient,
temporary failure, timeout, retry and duplicate protection; verification of webhooks and
delivered/delayed/bounced/rejected status; a controlled run of 100-300 messages measuring
delivery time, bounce rate and spam placement; confirmation that permanent failures and a
stalled queue alert fau@ewb-solutions.as; and storing only necessary delivery metadata locally,
avoiding durable storage of message content.

Sources are listed on #3410: Scaleway TEM plans/pricing/features, TEM SLA, Scaleway data
processing agreement, Scaleway subprocessors, and iliad's ownership of Scaleway. Nothing is
purchased or subscribed to; no account is created.

## Language policy — 9 September 2026

Erik instructed on #3411 and in chat: stick to English for technical language. Norwegian
renderings of technical terms are hard to read and the translations come out strange. This
applies to all internal technical work — architecture, ADRs, infrastructure, code, decision
documents, Favro cards and agent comments. It supersedes the earlier practice of writing Favro
card text and agent comments in Norwegian. Existing Norwegian card text is not retroactively
rewritten unless it is being edited anyway.

This is separate from the product's own language. The application remains a Bokmål product:
Erik confirmed on #3411 that "Bokmål is standard". The platform must also support Nynorsk and
translations generally, so localisation is an architecture requirement rather than a later
add-on: locale-aware content and UI strings, a defined fallback to Bokmål, and no assumption
that one FAU uses one language forever. Agents must not write Nynorsk content themselves —
Erik states translation will be done by someone else. Deliver the mechanism and the Bokmål
source strings; leave the Nynorsk text to a human translator.

## ADR-001 accepted — 9 September 2026

Erik accepted ADR-001 (docs/repo-container-contract.md, attached to #3411) at 09:12:03.607 UTC:
"ADR-001 accepted with the comments above to be handled." This makes the Rust/TypeScript repo
layout, the single application image, serve on port 8000, environment variables, the separate
migration job, healthchecks, logging/OTLP, local Compose and the verification requirements for
#3416/#3422/#3424 the accepted implementation basis. Acceptance is conditional on the two
comments Erik left on the same card being handled: translation/Nynorsk support, and the
question of moving from WireGuard to Tailscale. Both are tracked as their own cards. The
acceptance covers the document; no code, build or deployment is authorized by it.

## WireGuard to Tailscale — question logged 9 September 2026

Erik asks on #3411 whether a small Terraform module could switch from WireGuard to Tailscale,
and what that would require. Logged as an open question, not a decision. Initial reading of
infra-tools: WireGuard is not an isolated module but the network topology itself. It spans
shared-modules/wireguard_service and wireguard_client, bootstrap/network/vpn-router.tf and its
cloud-init template, k3s-agent, remote-servers, 1-bootstrap/vpn.tf, and the three x25519
keypairs FAU already generated and applied in stage 0. A swap therefore is not additive and
touches modules FAU consumes read-only. It also raises the European-ownership requirement,
since Tailscale's coordination server is a hosted dependency; Headscale is the self-hosted
alternative to compare. Assessment before any module work.

Correction, same day: the agent framed Tailscale's ownership as a blocker. Erik corrected this —
Tailscale is Canadian, and that is acceptable for FAU's own operational services. Ownership was
therefore not the deciding factor. See the decision below.

## VPN stays on WireGuard — 9 September 2026

Erik decided, after the initial assessment on #3440: stay with WireGuard for now, because it is
less work and lets the infrastructure work continue. Tailscale is not rejected on ownership
grounds — Erik corrected the agent's framing and states that Tailscale being Canadian is
acceptable for FAU's own operational services. The decision is about effort and cost, not
jurisdiction.

Reasoning kept for the record: WireGuard is already load-bearing across infra-tools
(wireguard_service, wireguard_client, bootstrap/network/vpn-router.tf and its cloud-init
template, k3s-agent, remote-servers, 1-bootstrap/vpn.tf) and FAU has already generated and
applied three x25519 keypairs in stage 0. Keeping WireGuard means no changes to modules FAU
consumes read-only, no dead stage-0 key material, no new hosted dependency, and no new secrets
in the Terraform inputs. Erik also notes a possible future saving if his other work standardises
on WireGuard rather than paying for Tailscale.

This is a "for now" decision, not a permanent architectural veto. If it is revisited, the open
questions from #3440 still apply: whether a Terraform provider covers the resources needed (auth
keys or OAuth clients, ACL policy, device approval, subnet router, exit node, tags), what must
move into cloud-init instead, subscription cost against the removed VPN router VM, the fate of
the applied x25519 keys, and self-hosted Headscale as the alternative. #3440 is archived with
this section as its history reference.

Scope note: see the supplier ownership policy below, which this decision prompted Erik to state
in general terms.

## Supplier ownership policy — 9 September 2026

Erik stated the rule the earlier "all European" requirement was standing in for. The requirement
is not uniform across every dependency; it follows whether a supplier is customer-facing.

- Customer-facing services must be European. This covers anything in the path of FAU users and
  their data: hosting, storage, database, transactional email, domains and DNS, and any service
  that processes, stores or transports personal data belonging to FAU members. The email decision
  on #3410 sits here, and so does the privacy work on #3426.
- FAU's own internal services may be non-European. Canadian ownership is explicitly acceptable
  here, stated in the context of Tailscale. American tooling is already in use and accepted:
  Erik uses American AI assistants, including Claude and ChatGPT, for this project's development
  and planning work.

The dividing line is the user and their data, not the company's convenience. An internal tool
that starts handling FAU member data crosses into the customer-facing category and needs a new
decision. This supersedes the flat "everything European" framing in earlier entries and in
#3409's original acceptance criteria; #3409 should map suppliers in these two categories rather
than as one list. No supplier is approved by this policy on its own.

## Agent operating model accepted — 9 September 2026

Erik accepted #3407 on 9 September 2026 at 09:34:17.211 UTC, ticking the review item and
commenting "Looks good, agree with extension for development." The reviewed model in
docs/agent-operating-model.md is the accepted basis: one board per stable role, one primary
owning role per card, the Inbox to Done workflow, Waiting versus Blocked, the pinned human
checklist and the evidence requirement before Done. The extension to development roles is
accepted, so implementation roles can be started.

Erik added a go-to-market idea in the same comment: sales and marketing build a list of all
schools with FAUs and their municipalities; development uses it to populate which schools an FAU
can sign up for; marketing then contacts the FAU at a named school with a link to a signup page
that prefills the public school information. This is tracked as #3441, which depends on the
register work on #3431 and feeds #3413, #3417, #3422 and #3423. Two constraints were recorded
rather than assumed: prefilled school information is a correctable suggestion, not verified
fact, and a per-school link must not carry personal data, grant membership, allow the register
to be enumerated or let anyone claim an existing FAU. The outreach itself is external
communication with personal data and needs Erik's explicit authorization plus a legal and
privacy check on #3427 and #3426; nothing has been sent.

## Supplier mapping split — 9 September 2026

Erik directed splitting #3409 along the two supplier categories. #3409 keeps the customer-facing
half, rewritten in English, and now covers hosting, object storage, database, transactional
email, domains and DNS, and any edge layer, all of which must be European. Internal tooling and
developer services moved to #3442, where they may be non-European; that card records GitHub and
GHCR, Favro, the American AI assistants in use, and monitoring endpoints, and writes down the
data boundary each must not cross. The escalation trigger is a supplier starting to handle FAU
member data, which returns it to #3409 and needs a new decision.

## Cloudflare and the edge layer — 9 September 2026

Erik asked for a European alternative to Cloudflare, and to first establish whether one is
needed at all, since Hetzner may cover the use cases. Logged as an open question on #3443, with
#3409 depending on it.

Two findings shape it. First, Cloudflare is not just a candidate: infra-tools' shared dns_record
module is built on the cloudflare/cloudflare provider and creates cloudflare_dns_record
resources, so using upstream DNS unchanged means using Cloudflare. Replacing it requires a
FAU-owned variant of that module, since the shared modules are consumed read-only. Second, the
existing stack already covers more than expected: TLS is terminated inside the cluster through
cert-manager with a cluster issuer, the ingress and wildcard-certificate modules and
ingress-nginx, with a Hetzner load balancer in front of nginx. The genuinely open functions are
therefore authoritative DNS, and whether a CDN, WAF or DDoS layer is wanted on top.

#3443 must settle the requirement per function before comparing vendors. For a Norwegian,
single-region, mostly authenticated document application whose heavy files already pass through
the authorization layer, a CDN may be unnecessary, which is the cheapest way to satisfy the
European requirement. Hetzner DNS is checked first as an already-accepted supplier; European
alternatives are compared only where a function is genuinely required. No domain purchase,
account creation or DNS migration is authorized, and the domain itself is still open on #3434.

## Infra-tools review accepted — 9 September 2026

Erik accepted #3406 on 9 September 2026 at 09:51:07.537 UTC, ticking the review item and
commenting "Looks good, accepted." The mapping in docs/infra-mvp-mapping.md is the accepted
account of what FAU reuses from infra-tools unchanged, what needs FAU-owned root configuration,
and which gaps the application must fill. The three-server topology stands.

Erik added: "Later we should go through and see which extra services, like slack and ntfy are
actually needed." This is a pruning question, folded into #3442, which now asks two questions per
internal service — is it needed at all, and who owns it — with need decided first. Neither Slack
nor ntfy is adopted, and notification routing overlaps with the alerting the telemetry modules
already provide and with the Favro watcher on #3436.

## Edge and DNS recommendation — 9 September 2026

Assessment done on #3443 while Erik was away; the decision is his and is not yet made.
Recommendation: drop Cloudflare, use Hetzner DNS, and buy no CDN, WAF or DDoS service for the
MVP. Full reasoning and sources in docs/edge-dns-assessment.md, attached to #3443.

The substantive point is that Cloudflare's functions are mostly already covered in-cluster by
cert-manager, the ingress and wildcard-certificate modules and ingress-nginx behind a Hetzner
load balancer, leaving authoritative DNS as the only missing function. Hetzner DNS provides it
free, authoritative, with the record types FAU needs, and works with a domain registered
elsewhere. A CDN is close to useless for this product because #3435 routes every file download
through the authorization layer with short-lived signed URLs, which a shared CDN cannot cache
without breaking tenant isolation.

One limitation requires an explicit decision: Hetzner supports DS records but not DNSKEY, so it
cannot sign a zone, and a Hetzner-hosted zone is unsigned. Recommended is to accept that for the
MVP and reconsider before production, with deSEC or the registrar's DNS as the European signing
alternative. This weighs more than usual because authentication is by magic link over email.
Unverified and recorded as such: Hetzner DNS zone and record limits (their limits page 404s) and
the exact scope of Hetzner's included DDoS protection.

## Localisation specification — 9 September 2026

Specified on #3439 while Erik was away, in response to his condition on ADR-001. Design in
docs/localisation-design.md, attached to the card. No Nynorsk text was written, per his
instruction that translation is someone else's task; the deliverable is the mechanism and the
Bokmål-source rules. Four decisions are open: whether Nynorsk must ship at MVP launch, the URL
strategy for public pages, whether to plan a third language, and who translates in which format.

Two previously accepted decisions become load-bearing and must be protected. ADR-001's stable
API error codes mean the server never returns display text, and #3412's audit model stores an
action code plus bounded metadata rather than rendered sentences. If either is relaxed,
localisation breaks and history freezes in the writing user's language.

Additive model changes proposed: account.locale, tenant.default_locale, an optional
document.language used for the lang attribute, and a locale captured on the outbox row at
enqueue time so a preference change or retry cannot alter an already-composed message. Documents
are never translated by the platform, and finalized snapshots keep the language they were written
in. The translator round trip is a file export and import, adding no supplier to #3409.

## Favro CLI attachment loss — 9 September 2026

Reproduced with a temporary diagnostic card (#3444, archived): the whole-description commands in
the favro CLI destroy a card's existing attachments. `set-desc`, `set-notes`, `set-result` and
`set-todo` each took a card from one attachment to zero, silently and with a success message.
`comment`, `tag`, `move` and `archive` are safe, and comments, ticks and description text survive
throughout.

This is why several cards showed a verified attachment and then lost it minutes later: the file
was attached first and a result block or checklist was written afterwards. Both the CLI's own
post-upload verification and a `favro get` check pass at the time and become stale on the next
description write.

Working rule, now recorded in the favro skill and CLAUDE.md: attach last, after every description
write on that card, and verify attachments again at the end of a batch of card edits rather than
immediately after upload. Worth reporting upstream as a CLI bug; the underlying cause looks like
the description write replacing the card body that holds the attachment references.

## Stable error codes and Nynorsk timing — 9 September 2026

Erik decided on #3439: "I like stable error codes, so we go for that", and "Nynorsk is just for
design right now, we stick with Bokmål for now since I want a proper translation when we launch
that."

Stable error codes are therefore a decision, not a proposal, and bind ADR-001 and #3412: the API
returns codes and parameters rather than display text, and audit stores action codes rather than
rendered sentences. Localisation depends on both.

Nynorsk is designed for but not shipped. The MVP deliverable is the mechanism plus a complete
Bokmål catalogue; no nn-NO catalogue is created, not even a stub, and the language switcher stays
hidden while one locale exists. Translation happens at launch, by a human translator, so the
export and import round trip is proven with Bokmål as both source and target and no agent
authors Nynorsk. #3439 is therefore not blocking; the remaining public-page URL question can wait
for a second locale, provided #3422 and #3423 keep a path prefix cheap to add.

Erik also corrected an overstatement of mine: I had argued that Bokmål audit history would be
"unreadable" to a Nynorsk reader. It is not — Nynorsk readers read Bokmål without difficulty, and
he confirms this is not a deal breaker. The language-neutral rules stand on narrower grounds:
they are what makes a genuinely different third language possible later, for example one spoken
by immigrant families at a school, and rendering from codes is the cleaner contract anyway.

## Registrar and DNS at Domeneshop — 9 September 2026

Erik proposed domene.shop (Domeneshop), the registrar he already uses, with records set up by
hand since the IPs are static. Assessed on #3443 and recommended; his reasoning holds. Domeneshop
is a Norwegian company, so European for the customer-facing category, and taking registrar and
DNS from one supplier is one record on #3409 rather than two. It has a documented REST API with
full CRUD on DNS records and a maintained Certbot plugin; the API record models cover A, AAAA,
CNAME, MX, SRV and TXT, which is what the apex, www and the #3410 email records need. With static
load balancer IPs the record set is roughly six to eight records that almost never change, so
manual maintenance is reasonable and keeps a community Terraform provider out of the path.

The assessment found a second Cloudflare dependency in the inherited stack. Besides the
dns_record module, cert-manager's cluster issuer defines an ACME issuer whose dns01 solver is
Cloudflare-specific, holding a Cloudflare API token as a Kubernetes secret. DNS-01 is what
wildcard certificates require, and the wildcard-certificate module requests *.<domain> plus the
apex; cert-manager has no native Domeneshop solver. Recommended resolution: skip wildcard
certificates and issue HTTP-01 per hostname, which the ingress module already supports, which
suits ADR-001's single-origin path layout, and which removes the Cloudflare token from the
cluster.

Open, on Erik: whether Domeneshop can create CAA records and whether it offers DNSSEC — if it
signs the zone, the Hetzner DNSSEC limitation disappears. Open as a product question: whether FAU
will ever serve per-tenant subdomains, which would force wildcard certificates and DNS-01 back
into the design and should be settled before the certificate path is built.

## Favro CLI bug handed off — 9 September 2026

The attachment-loss defect is written up as a paste-ready GitHub issue in
docs/favro-cli-attachment-bug-issue.md, attached to #3445, so Erik can file it against the
favro-cli repository and have it fixed there. The card body is the issue verbatim. It carries the
reproduction, the four PUT /cards call sites in src/main.rs, the finding that attachments are a
first-class card array rather than description links, three ranked fix options, a
verify-after-write guard, and acceptance criteria including retirement of the downstream
workaround notes once released.

## Path addressing, DNSSEC and certificates settled — 9 September 2026

Erik approved the #3443 recommendation and closed its open items.

Certificates: skip wildcard certificates, issue per hostname over HTTP-01. Combined with his
decision that FAU-er are addressed by path rather than subdomain — example.no/kommune/school-name,
"no subdomains" — this is the permanent design rather than a stopgap, since there is never a
`*.example.no` to cover. It also removes the Cloudflare-specific DNS-01 solver, and with it the
Cloudflare API token, from the cluster.

DNSSEC: handled by Domeneshop, verified from a live zone transfer Erik supplied for one of his
domains rather than from marketing copy. The zone carries a KSK and two ZSKs on algorithm 15
(Ed25519), RRSIG over every record set including the DNSKEY set, and NSEC3PARAM, served from
ns1/ns2/ns3.hyp.net. Nothing for FAU to configure, and no second DNS supplier such as deSEC.

CAA: optional hardening, deferred to when the domain exists on #3434. It restricts which
certificate authorities may issue for the domain; absent, any public CA may. FAU will use exactly
one CA, and the signed zone means the record cannot be stripped in transit, so it is worth adding
then. Domeneshop's documented API record models do not include CAA, so it may need to be set in
the panel or skipped.

Three consequences of path addressing, assigned rather than resolved:

1. Root path segments are now a shared namespace with ADR-001's reserved `/app`, `/api/v1`,
   `/assets` and `/health`, and with any locale prefix from #3439. Recommended ordering is the
   outer form, `/nn/baerum/skole`, with unprefixed paths serving Bokmål so links never change.
   Not blocking while Bokmål is the only locale, but #3422 and #3423 should keep it cheap to add.
2. Slugs need defined rules and stability, tracked on #3441: explicit slug columns for
   municipality and school rather than deriving from display names, uniqueness enforced per
   municipality, and redirects rather than 404s when municipalities merge or schools are renamed
   or closed — #3441's outreach emails will put these URLs in front of people months later.
3. A page per school is a public, indexable surface, so #3423 must treat these as real pages with
   their own titles rather than as internal routing.

## URL scheme (ADR-002) — 9 September 2026

Erik asked to rethink the URL scheme after the path-addressing decision on #3443 exposed
root-namespace and slug-stability problems. Designed with him in conversation the same day and
written up as ADR-002 in docs/url-scheme.md, attached to #3446, pending his review.

Decisions: public pages carry school slugs and the workspace never does, with the tenant resolved
from the resource so renames cannot touch bookmarks or sessions; identity is a UUID we assign,
with kommunenummer, organisasjonsnummer and names as versioned attributes rather than keys;
document IDs are UUIDs so a shared link cannot be valid in two FAU-er at once; two link forms,
pretty and mutable for humans and search, permanent and opaque for anything we send; the
municipality segment is `<kommunenr>-<navn>`; correct transliteration ae, oe, aa, lowercase
canonical; schools addressed by name alone; Bokmål unprefixed with any locale prefix outermost;
and unknown schools submitted from the municipality page with a link to the establishing decision,
a kept copy of that document, an email to us for manual verification, and a pretty slug issued
only after verification so no one can squat a real school's readable URL.

The Norwegian detail drove the design, and Erik supplied the cases that killed three of my
proposals in turn. Municipality names are not unique — Herøy names two municipalities.
Kommunenummer are county-prefixed and renumbered by county reform. Both are also reused: the 2017
merger of Sandefjord (0706), Andebu (0719) and most of Stokke (0720) produced 0710 Sandefjord, and
0716 belonged to Våle before it was given to Re in 2002. Name-only slugs, kommunenummer-as-identity
and a disambiguation-page fallback each failed against those facts. Erik's resolution was the
compound key: number and name change independently but never together, so the pair is unique in
practice and fails safe, because a stale pair cannot match a current pair.

Two of his additions closed real gaps: copies of every linked decision document are kept, since
evidence behind a dead link is no evidence and municipal sites reorganise constantly; and manual
school submissions email us so a human verifies before the pretty slug is issued. The design also
records that fetching a user-supplied URL is server-side request forgery unless constrained, with
the specific constraints listed for the security work.

No implementation is authorized. Consequences are routed to ADR-001, #3441, #3413, #3417, #3422,
#3423, #3439, #3412, #3426 and #3410.

## Document ingest security — 9 September 2026

Two requirements from Erik, added to ADR-002 and owned in detail by #3447.

**Strip executable code before storing or serving.** PDFs are programmable — JavaScript actions,
`/OpenAction`, additional-action triggers, `/Launch`, embedded files, RichMedia and XFA all
execute or fetch on open in some readers — so every document is sanitised on ingest, whether it
was fetched from a municipality or uploaded by an FAU under #3419 and #3435. Plain `/URI` link
annotations are kept deliberately: a phishing vector rather than code execution, and stripping
them would break real references inside minutes.

The wrinkle worth recording: sanitising changes the bytes, so a sanitised file no longer hashes
to what the municipality published, which would destroy the provenance value that motivated
keeping a copy. Resolution is two copies — the original quarantined with its hash and never
served, the sanitised derivative served to reviewers and users — with the audit entry naming what
was stripped. Type detection is from magic bytes, never the extension or the origin server's
declared type. Accept list for MVP is PDF, plain text and Markdown; Office formats are refused,
macro-bearing ones explicitly, and SVG is refused as scriptable. Failure quarantines rather than
passes through, and a school submission still completes. Malware scanning is a separate control
and remains an open item.

**Automatic fetching is allowlisted to official sources**: `udir.no`, `*.udir.no` and
`*.kommune.no`, matched on host by exact or registrable-suffix match after punycode normalisation,
never by substring. The important refinement is that the allowlist governs fetching rather than
admission. Many Norwegian municipalities publish minutes through vendor-hosted innsyn and
møtekalender portals, and some municipalities and county bodies use custom domains, so a strict
two-domain rule would reject a large share of legitimate submissions. A non-allowlisted URL is
accepted with the submission but not fetched; the reviewer who already receives the verification
email retrieves it and either extends the allowlist or attaches the document. Extending the
allowlist is a review action, never submitter-triggered.

The allowlist is an SSRF control, not a content-trust control — an allowlisted municipal site can
still serve a hostile PDF — so the fetcher keeps its address and redirect restrictions even
inside the allowlist, and sanitisation applies to allowlisted sources exactly as to any other.

## Favro CLI 0.2.1 installed — 9 September 2026

The attachment-loss defect written up on #3445 is fixed upstream in 0.2.1. Erik cloned the
upstream repository into the workspace mount; the crate was copied over the vendored copy at
.agents/skills/favro/favro-cli together with upstream's updated SKILL.md, references/cli.md and a
new tests/live_attachments.py, and the clone was removed. LICENSE verified byte-identical to the
repository root, which the CLI README makes a release requirement.

Built with cargo 1.98.1 and installed to /usr/local/cargo/bin, which precedes the image's
/usr/local/bin on PATH, so `favro --version` reports 0.2.1 immediately. **This install is not
persistent** — /usr/local/cargo is not a mounted volume, so a container recreation restores the
image's 0.2.0. Making it durable requires a Dockerfile.agent rebuild on the host, which is a
host-side action by the operating rules.

Verified against the original reproduction: attach, then each of set-result, set-notes, set-todo
and set-desc, with the attachment surviving every one, where 0.2.0 dropped it on the first.
Repeated with two files on one card and across a lane move. Diagnostic card #3480 archived; the
0.2.0 reproduction remains on archived #3444.

Correction to the agent's own root-cause analysis in the #3445 issue text: it concluded the loss
was server-side on the card PUT and explicitly not the CLI's Markdown handling. That was wrong. A
Markdown PUT rebuilds Favro's attachment list from image nodes in the description, including
non-image uploads, and neither an `attachments` nor an `addAttachments` body field works — which
is precisely the question the card had flagged for investigation before implementation. The fix
re-emits each existing file as an image node with its remote fileURL ahead of the user text, then
re-reads the card and fails loudly if a name or count is missing.

Downstream notes updated accordingly. The "attach last, verify at the end of a batch" workaround
is retired from the skill and CLAUDE.md so it does not outlive the bug; what remains is the
durable rule that an attachment claim must be checked against `favro get`. Two behaviours replace
it: archive and move-board verify attachments too, and whole-description writes are
read/modify/write, so one card's description and its attachments must not be edited concurrently.

Not done: upstream's tests/live_attachments.py was not run, because it creates cards on whichever
boards it is pointed at and that would mean test cards on the real project boards. It self-archives
on success and failure, so it can be run when a deliberate target is chosen.

## Terraform state: remote, versioned and destruction-proofed — 9 September 2026

Erik chose the order of work himself: prove that versioning retains versions first, then make
accidental destruction impossible. The off-Hetzner copy is parked until those two are done.

The starting point turned out to be better than every document claimed. The migration of stage-0
state to a remote S3 backend in the `fau-tfstate` bucket had already been carried out on
8 September at 12:56 UTC, but it was recorded in no document and on no card. `docs/stage-0-readiness.md`
stopped at the investigation, this file still said "State backup remains open", #3438 still listed
the backup as the one thing left, and CLAUDE.md still said state lived only in the `infra-runtime`
volume. All four were a day stale; all four are now corrected.

**Versioning retains versions — verified, not assumed.** Proven on a scratch key, never the state
key: three versions coexisting after two overwrites, a plain delete producing a recoverable delete
marker, and byte-exact recovery by version id. Every probe artifact was purged afterwards and the
bucket verified back to one object.

Two findings came out of it. First, a read timeout is not a failed write on this endpoint — two
PUTs exceeded a 30-second timeout and both landed anyway, which matters because a blind retry
against `use_lockfile` looks like a lock conflict rather than a completed write. Second, the state
object's only version is `VersionId: null`, written before versioning took effect, and the
overwrite path for a null version cannot be tested now that versioning is on. A server-side copy
to `_snapshots/2026-09-08T125636Z-terraform.tfstate` removes the dependency on that assumption.

**Destruction now takes intent.** A bucket policy denies `s3:DeleteObjectVersion` — the only
operation that destroys data permanently — plus `s3:DeleteBucket` and `s3:PutBucketVersioning`.
Plain `s3:DeleteObject` stays allowed, because with versioning on it only creates a recoverable
marker and Terraform's lock release needs it.

The deliberate omission is `s3:PutBucketPolicy`: denying it would make the guard unremovable and
a mistake unrecoverable. Leaving it means undoing the protection requires a deliberate policy
change, which is precisely the bar Erik set — accident, not malice. Object Lock would be the
stronger control, but S3 and Ceph both require it at bucket creation, so it would mean a second
state migration; not proposed.

Verified against our own owner credential, since Hetzner issues no narrower one: a versionId
delete returns 403 even for a non-existent key, a no-op versioning write returns 403, and
`terraform plan` still acquires the lock, refreshes twelve resources, reports "No changes" and
releases. Two caveats are recorded rather than smoothed over. The `s3:DeleteBucket` deny is
unverified — bucket deletion returned `409 BucketNotEmpty` from what looks like a layer ahead of
the policy engine — though deletion is still blocked transitively, since emptying the bucket needs
the denied operation. And every Terraform run now leaves an unpurgeable lock version plus delete
marker, kilobytes per thousand runs, accepted as the price of the guard.

**What this does not cover.** One provider, one project, one credential pair, no copy outside
Hetzner. State survives accident, not loss of the account. That remains the open decision, and it
is a key-custody question rather than a mechanics one, because state holds the admin SSH key, the
three WireGuard keys and the age key in clear text.

## No off-Hetzner state copy required — 9 September 2026

Erik closed the last open point on #3438 in a card comment: "not an issue, in case of loss I can
create new keys in the hetzner console. and if we lose the account we probably lose everything
else as well. but in the future I can save out the keys in proton pass."

So stage 1 proceeds on accident protection alone — remote versioned state with permanent deletion
denied by bucket policy — and no encrypted copy outside Hetzner is built now. The reasoning is
that the secrets in state are regenerable rather than irreplaceable, and that a lost Hetzner
account takes the infrastructure with it, which makes a state copy the smaller problem. Proton
Pass is the intended future home for the keys, as a manual export rather than a mechanism to
build.

**The condition that ends this decision.** Erik narrowed it the same day, correctly: registering
the admin public key in Hetzner Robot is not a trigger, because Robot is reachable by several
means — the Robot web interface, the Robot API credentials, and the rescue system, any of which
can install a replacement key. The admin SSH key stays regenerable, so losing it costs a re-run,
not access.

The one event that makes state genuinely irreplaceable is GitOps secrets being encrypted to the
SOPS age key. Regenerating an age key does not recover anything already encrypted to the old one;
that needs the plaintext from somewhere else. Whoever reaches that milestone re-opens the
question rather than inheriting this decision unexamined.

Erik's stated resolution removes even that: storing all the generated keys in Proton Pass. Doing
it before GitOps secrets exist closes the risk permanently and makes the trigger moot.

This supersedes the earlier "a backup must exist before stage 1" position recorded on 8 September
under "Bucket protection and plan retention". #3438 closes.

## Node choice: start on three CX23 — 9 September 2026

Erik chose three CX23 for the initial cluster, having spotted that the cost-optimized category is
now orderable in hel1 at EUR 5.49: "we can possibly order CX33 later, but start with CX23 if they
will fit our needs for now." This supersedes both the existing CPX12 + 2 × CPX22 plan and the
CPX12 + 2 × CPX32 recommendation in docs/hetzner-node-assessment.md, and it retires the earlier
note that the cost-optimized category was unavailable.

The comparison that settles it: CX23 and CPX22 have identical 2 vCPU and 4 GB, with CX23 at 5.49
against 19.49 and half the local disk. Paying CPX22 prices for CX23 specifications had no
justification. Three identical CX23 also give 12 GB total against the existing plan's 10 GB and
remove the asymmetry where the VPN and master node had only 2 GB. Total cost lands near EUR 47 per
month net against about EUR 113 for the CPX32 recommendation.

**Fit, from declared resources rather than measurement.** Explicit memory requests across every
module total roughly 1.85 GB, dominated by Grafana's 1 Gi, against roughly 9 GB usable across
three 4 GB nodes. Everything schedules with room. What is not covered is that Loki, Tempo,
VictoriaMetrics and Alloy declare no resources and inherit chart defaults, and their real
consumption tracks ingestion rather than requests, so 4 GB per node remains a measured question
rather than a settled one. Nothing is live and there is no data yet, which is what makes starting
small reasonable.

**One required change before stage 2.** `psql-cluster` leaves `db_memory_request` null by default,
which upstream documents as leaving the database pods BestEffort and "the first thing evicted
under node memory pressure". On 4 GB nodes that makes PostgreSQL the first casualty of a memory
spike. FAU's root must set `db_memory_request`, starting around 512Mi, and must leave
`db_memory_limit` null, since a memory limit converts pressure into an OOMKill of the primary.

**The exit is cheap, which is the point.** A server type change is an in-place rescale plus a
reboot: CX23 to CX33 stays x86 and grows the disk 40 to 80 GB, the permitted direction, and data
sits on network volumes so nothing migrates. CX33 and CX43 are priced and supported but out of
stock in all three EU datacenters today, so that upgrade waits on Hetzner inventory. Choosing x86
deliberately keeps it open: ARM CAX21 would give 8 GB per node for EUR 10.49, but rescaling cannot
cross architectures and no ARM image audit has been done.

**Exact PVC costing dropped.** Erik accepted the EUR 14.30–15.35 volume estimate the same day:
"accept the estimate, no need for exact PVC numbers." So `loki` and `tempo` will not be pinned and
`helm` will not be added to the agent image for costing purposes, and the first invoice becomes
the authority on volume cost. Pinning those two charts remains worthwhile for reproducible
deployments, but as stage 2 hygiene rather than as a cost exercise. This closes #3408.

## Stage 1 applied: three CX23 and a k3s cluster — 9 September 2026

Erik authorized the stage-1 apply after reviewing the plan, and it was brought forward
deliberately rather than scheduled. He corrected an assumption of the agent's that mattered: the
agent read CX23 being available in all three EU datacenters as a sign of abundance, when Erik had
watched the category flicker — CX13 available on 8 September, nothing at all earlier on
9 September, CX23 available now. Simultaneous availability across datacenters is a restock, not
supply. On that evidence the capacity was claimed while it existed.

Ordering by hand was ruled out on a concrete technical ground, not on preference:
`hcloud_server.user_data` is a rendered cloud-init template and forces replacement, and cloud-init
only runs on first boot. A console-created server, once imported, would either be destroyed by the
next plan or left permanently unconfigured. The only order that holds capacity usefully is a
Terraform apply — which is also safe to attempt under time pressure, because Terraform creates the
servers early and does not roll back on failure, so a failed provisioning tail still leaves the
servers standing.

Applied: 31 added, 0 changed, 0 destroyed, no errors, and a fresh plan reports "No changes".
Three `cx23` in hel1 — `master`, `vpn-router`, `cworker-1` — plus the `lb-fau` load balancer, the
`fau-backend` network, three routes, four firewalls, a placement group and three WireGuard
keypairs. All three nodes joined and are Ready on k3s v1.35.2+k3s1. Running cost starts at about
EUR 25.46 per month net. Full record in docs/stage-1-readiness.md.

Four decisions were taken inside FAU's own stage-1 root, with upstream modules untouched. All
three servers are cx23. Upstream's `extra-firewall`, which opened TCP 6000-6001 to the whole
internet on the worker as a plumbing demo, is removed along with its demo node label. State is
remote in `fau-tfstate` under `1-bootstrap/terraform.tfstate` rather than local as upstream ships
it, because stage 1 generates three more WireGuard private keys into state and they deserve the
same versioned, deletion-denied protection as stage 0. And `infra_tools_path` is relative here,
unlike stage 0's absolute path, because the `bootstrap/*` modules reach siblings with `../` and
Terraform rejects that as escaping a module package when the path is absolute.

**Two findings worth carrying forward.** Three kube-system pods sit Pending, which is expected
rather than broken: every node carries
`node.cloudprovider.kubernetes.io/uninitialized=true:NoSchedule`, the external cloud-provider
handshake that the Hetzner CCM clears when stage 2 installs it.

And the private-network question from the pre-0 guide can now be stated exactly. WireGuard from
the agent container is impossible today — `/dev/net/tun` does not exist and `CapEff` is all
zeroes, so `wg` and `wg-quick` are installed but inert; enabling it needs `devices` and
`cap_add: NET_ADMIN` in `docker-compose.yml` plus a host-side container recreation, which ends the
agent session. The SSH bastion through the vpn-router needs no capabilities and already works:
every verification of this cluster was done that way. So stage 2 has a real choice — a free SSH
port-forward now, or a cleaner WireGuard fabric at the cost of a host-side change.

## Agent WireGuard access granted; stage 2's network choice closed — 9 September 2026

The choice recorded above — SSH bastion now, or WireGuard at the cost of a host-side change — was
decided in favour of WireGuard, and Erik carried out the host-side work the same day. The agent
service in `docker-compose.yml` now has `devices: /dev/net/tun` and `cap_add: NET_ADMIN`, and the
container was **rebuilt** rather than bare-recreated, as the handoff asked.

`cap_add` alone would not have been enough, and the image accounts for that: capabilities land in
the bounding set, which only root-run processes inherit, while the session runs as `claude`.
`Dockerfile.agent` installs a narrow rule at `/etc/sudoers.d/50-wireguard` permitting exactly
`/usr/bin/wg-quick` and `/usr/bin/wg` as root, nothing else. `wg-quick` runs as root under sudo,
so everything it calls internally — `ip`, `iptables`, `resolvconf` — needs no further rules.

**Verified from inside the container, 9 September.**
`sudo wg-quick up /infra-runtime/infrastructure/.config/fau.conf` brings up interface `fau` at
10.0.200.2/32, MTU 1420, routing 10.0.0.0/14. The peer handshakes within seconds at
`2.29.26.108:51890`, which Erik confirmed against the Hetzner Cloud console as the vpn-router's
public address, with a 25-second persistent keepalive. `kubectl` then reaches
`https://10.0.1.250:6443` directly, with no port-forward and no TLS exceptions: all three nodes
Ready on v1.35.2+k3s1. The three kube-system pods are still Pending on the `uninitialized` taint,
unchanged and still stage 2's job.

**The tunnel is per-container-lifetime.** Nothing raises it at container start, so it must be
brought up once per container, and any recreation drops it. The procedure is in the
`.agents/skills/fau-vpn/SKILL.md` rather than in prose, so it costs no context until it is
needed. The SSH bastion through the vpn-router remains a working fallback and still needs no
capabilities.

**A second defect closed as a side effect.** The rebuild baked Favro CLI 0.2.1 into the image at
`/usr/local/bin/favro`; the non-persistent `cargo install` copy is gone. A container recreation no
longer falls back to 0.2.0 and its attachment-destroying description writes.

Privilege note, kept in the open rather than buried: this container already bind-mounts the Docker
socket, which is root-equivalent on the host. `NET_ADMIN` and a two-binary sudo rule are marginal
next to that, and were accepted on that basis.

Erik accepted #3482, the stage 1 outcome card, the same evening, and it moved to Done.

## Identity bought, data encrypted, and the recovery contact — 10 September 2026

Two requests that looked like one: buy authentication rather than build it, and encrypt user data
so a leak is not a disclosure. Full design in docs/identity-and-encryption.md (ADR-003). The
decisions, and the reasoning that changed along the way:

**An external magic-link provider does not produce user-held keys.** Magic links prove control of
a mailbox; there is no secret the user knows, so there is nothing to derive a key from. Erik's
initial framing was that using Zitadel would mean we could not decrypt. It does not follow, and
the recovery requirement settles it in the other direction: asked whether a new committee must be
able to read last year's documents when the previous one vanished without handing over, Erik chose
that recovery must always work. That is only possible if we hold a key path. So we do, and the
trust model says so in the words we will use publicly - restricted and audited access, never "we
cannot see your data".

**What the encryption is actually for**, in Erik's own framing: if our data is leaked it should be
encrypted, and getting at files should require adding an email, verifying it, and downloading -
per FAU, every time. That is a statement about attacker cost, and it drove two hard constraints. No
bulk unwrap: the master key lives only in a small key service that unwraps one FAU's key per
authenticated session, logged and rate-limited, so a compromised backend has to ask once per FAU
in the open. And no cross-tenant read path at all - no support endpoint, no export tool - because
a quieter path is the one an attacker would use.

**The encryption boundary** includes document titles and original filenames alongside bodies,
change sets and uploaded bytes. A title like "Klage på lærer Hansen" discloses as much as the file
it names. Member emails, names, roles, dates and audit event types stay plaintext. Accepted
consequence: no cross-FAU search exists, including for us.

**Zitadel Cloud, Swiss region, as a time-boxed exception.** Erik established that the Swiss entity
is a subsidiary of Zitadel LLC in California, so a customer-facing service is not European-owned
and the supplier rule is not satisfied. Accepted for now on two grounds - the provider holds
addresses and authentication events, never content, and passwordless authentication makes the
provider unusually cheap to replace - and conditioned on the identity provider being replaceable
by design: canonical user records ours, subject ids in a mapping row, standard OIDC only, no roles
or permissions in the provider, and a migration describable as configuration plus a re-mapping job.
Ory Network is the standing German-owned alternative.

On pricing, Erik's reading was that 30-day sessions reduce billing. Half right: Zitadel counts a
daily active user as anyone who authenticates **or refreshes a token** that day, so long sessions
do not make an active member free - they make an inactive one free, with registered users
unlimited on every tier. The cost driver is days of use, which for FAU parents is a few per month.
Two rules follow: never refresh tokens in the background, and one machine account rather than one
per service. The free tier's metric is ambiguous between the pricing page and the Pro tier wording
and must be confirmed in writing before anything depends on it.

**The recovery contact** replaces the earlier idea of us temporarily holding an admin role. Each
FAU chooses us or a school representative, verified as principal or inspector by register-derived
domain email plus a recorded manual title check. The seat cannot read documents or audit and
cannot seat itself; it can only initiate adding a member, after step-up authentication. Erik's
insight that principals and inspectors are the stable institutional anchor is what makes this work,
and a security review from Codex sharpened it further: because we hold the keys, granting
membership is pure authorization, so nobody needs content access in order to restore someone
else's. That removed the exception to #3412 the earlier design would have required - the rule that
handover-only roles get no document access now stands unmodified and applies to us too.

Accepted residual risk, stated rather than engineered away: a malicious recovery contact can seat
an unauthorised person, who has access until the misuse is noticed.

**Notification** is what keeps the window short. Members are emailed when an invitation is created
and again when it is accepted, with a 14-day login banner and a permanent audit entry. When the FAU
has no members left - the case the seat exists for - notice goes to the last known member addresses
and to the second party, the recovery option that did not initiate. We remain the second party for
every FAU even after vacating the seat: oversight without capability. This requires retaining
lapsed members' addresses past their role, 24 months proposed, which is a DPA clause rather than a
quiet implementation detail.

**Correction the same day, and the constraint it exposed.** Ory Network was recorded above as the
German-owned alternative to Zitadel. Erik established that it is US-owned too. No European-owned
provider has been identified to replace it, and the documents now say that rather than naming a
candidate: a managed-Keycloak route was considered and dropped, partly because Keycloak has no
built-in passwordless email authentication and supplying it means a community-maintained plugin,
which is the same "anything we own we maintain" problem that motivated buying identity at all.
Finding a European provider is now its own open item. The wrong Ory entry is kept as a visible
correction, because "it sounds European" is the reasoning the rule exists to prevent.

That produced an architectural correction worth more than the provider question: the requirement
is **passwordless authentication by email**, not the magic link specifically. A clickable link and
an emailed one-time code satisfy it equally, and stating it at the mechanism level was silently
narrowing the provider field. There is also a practical argument for codes - mail security
scanners follow links to inspect them and can consume a single-use magic link before the
recipient clicks, which is common in exactly the school and municipal mail systems FAU members
use, and presents as "this link has already been used". The mechanism is now an implementation
choice per provider and must not leak through the identity abstraction.

**Portability rules, fixed now rather than later.** A review from Codex on 10 September 2026
sharpened decision 3 into binding implementation rules, on the reasoning that they cost little
applied from the start and are close to unaffordable retrofitted - so they do not wait on knowing
the destination. Every person gets an internal UUID; the provider's `sub` is never a key. Mappings
are (issuer, subject) to internal user id, and **an internal user may hold several at once**,
which is what lets two providers run concurrently instead of forcing a cutover. Tenancy,
memberships, roles, recovery seats and anything gating a key unwrap stay in our database, never in
provider organisations or vendor claims. Audit is authoritative in our infrastructure. Only OIDC
Authorization Code Flow with PKCE. Issuer, endpoints, client ids and claim mappings are
configuration. `acr`, `amr`, `auth_time` and recent re-authentication reach the application
through our own abstraction, because coding against one provider's interpretation of step-up is
silent lock-in. The mapping and minimum profile are exported on a schedule, since an exit that
depends on the provider's API working on the day we leave is not an exit.

Migration is therefore an overlap, not a cutover: add the second issuer, keep both live, enrol
members into the new provider as they appear, link the new identity to the existing internal user
as an additional mapping row, re-enrol any second factor, and disable the old provider only after
the window closes. Passwordless authentication makes this nearly free for ordinary members, who
re-enrol by logging in. The corollary is recorded: **adopting passkeys later ends that**, because
passkeys bind to the provider's relying party and every member would have to re-enrol
deliberately. Passkey adoption is now a trade against the exit, not a pure security win.

On the free tier, Codex's arithmetic assumes the stricter reading of Zitadel's pricing and is
worth recording: 100 units summed across a month means three people active every calendar day
consume about 90. That makes the free tier a development tier rather than a pilot tier, and a
single machine account spends roughly 30 by itself. Service account activity gets monitored rather
than assumed free. The ambiguity between the free and Pro tier wording still needs confirming in
writing.

Erik also noted that a public school is legally required to have an active FAU, so an FAU with no
members is an anomaly rather than a normal end state. The empty-FAU recovery rules stay specified
and tested regardless: the path nobody expects is the one nobody writes code for.

## Stage 2's CCM pass applied and accepted — 10 September 2026

Erik authorized the apply on #3483 with "approved, proceed" and accepted the result the same
morning. The saved plan from 9 September was regenerated before the apply rather than trusted,
and came out byte-identical: 6 to add, 0 to change, 0 to destroy, the same six resources. Apply
added 6 with no errors, the `node.cloudprovider.kubernetes.io/uninitialized` taint cleared on all
three nodes, the three kube-system pods that had been Pending for thirteen hours went Running, and
a fresh plan afterwards reported no changes.

The rule the pass established, and the reason it is written here rather than only in the readiness
document: **a saved plan is regenerated, not trusted, once anything could have moved.** Terraform
refuses a plan whose state has advanced, but it cannot detect a cluster changed underneath it, so
the guarantee has to come from re-planning and reading the result. The consumed plan file was
deleted afterwards for the same reason — a spent `.tfplan` sitting in a stage directory is only a
stale-apply hazard.

One documented prediction was wrong, in the harmless direction, and is recorded as a correction
rather than quietly fixed: the CSI node DaemonSet does schedule on `vpn-router`, because
`hcloud-csi-node` carries blanket `operator: Exists` tolerations for `NoSchedule` and `NoExecute`.
The prediction came from reading the node's taint without reading the DaemonSet's tolerations. A
CSI node plugin on every node is the intended shape, so there was nothing to undo.

Scope was unchanged: the `cluster` module only. cert-manager, flux, telemetry and cloudnativepg
each still wait on their own open decision, so the pass unblocks nothing on that list. What it
unblocks is anything needing a working scheduler, Hetzner load balancer integration or persistent
volumes.

## Alerting endpoint accepted: Healthchecks.io, with Signal as the transport — 10 September 2026

Erik accepted the shape proposed on #3442, which answers the question he asked on 9 September —
an alert that reaches him when the cluster fails, without Slack. Alertmanager routes by severity
to two destinations: **warning to email**, through the managed European provider still being chosen
on #3410, and **critical to Signal**, via Healthchecks.io. The always-firing Watchdog alert pings
a Healthchecks check on a schedule, and the ping *stopping* is the alarm — that dead man's switch
is the only mechanism that can report the cluster or the host being gone, since a dead cluster
cannot send an alert about itself. It reaches Signal too, because total failure is by definition
time-critical.

Critical alerts use the same ping URL rather than a second supplier: Alertmanager's webhook
receiver POSTs to a check's `/fail` endpoint, Healthchecks treats the check as down and notifies
over Signal. One account, one phone path, and the dead man's switch comes free from the same check
model.

Three properties of the Signal integration are accepted with the decision rather than discovered
later. Signal has no incoming webhooks, so Healthchecks runs a real `signal-cli` client and talks
to it over JSON-RPC; the first message must be sent from the account's own side or Signal
rate-limits it; some accounts have needed a manual CAPTCHA step before notifications start; and
delivery takes seconds rather than being instant. None of that disqualifies a critical-only
channel, but it makes first setup a hands-on step rather than pure Terraform.

**Signal itself is US-owned** (Signal Technology Foundation), so the transport is non-European even
though Healthchecks.io is Latvian. That is permitted under the internal-supplier rule of
9 September — an internal service carrying scrubbed operational content — and it is recorded as a
choice rather than something that slid in. The boundary rule is stricter than for other internal
suppliers, and it extends to metadata: alert bodies must not carry user identifiers or audit
content out of the cluster, **and neither must check names**, because the check name is what
reaches the phone.

One item stays open and is a test rather than a decision: how much of the alert body survives into
the Signal message. A Healthchecks notification is built around the check name, and the POSTed body
may or may not be included. If it is not, the design still holds but the check names have to carry
the meaning — one check per alert class rather than one check for everything. That is ten minutes
of testing before Alertmanager is wired, not a reason to revisit the shape.

## Version control: one repo, and the two ignore rules that were wrong — 10 September 2026

Until this date **nothing in `/workspace` was under version control.** No `.git` existed anywhere
in the workspace or in `/infra-runtime`, so a month of planning - 19 documents, three Terraform
stage roots, the Compose file and `Dockerfile.agent`, the two agent skills - lived only in a single
Docker volume with no history and no copy off the box. Terraform *state* was versioned in
`fau-tfstate` with permanent deletion denied, while the decisions and configuration that produced
it had no protection at all. Erik ran `git init` on the host on 10 September; the agent does not
run git operations from inside the container.

**One repository, not several.** This is not a new decision: ADR-001, accepted 9 September,
specifies a single application repo whose layout already contains `infrastructure/`, `gitops/` and
`docs/`. The application code that does not exist yet joins this workspace rather than the reverse.
The reasoning that keeps it one repo at FAU's scale: the decision record constantly cross-references
the configuration it describes - `docs/stage-2-readiness.md` against `infrastructure/2-cluster` is
the pairing that went stale and had to be corrected twice in one day - and splitting them lets the
two halves drift silently. One clone per agent session also means an agent cannot read a decision
from one commit against Terraform from another. Splitting by language is specifically ruled out,
because ADR-001 builds one image from both halves and requires `api/openapi.yaml` and the generated
frontend types to be validated together in CI.

**Deferred, not decided: a separate `gitops/` repository.** If flux image automation is enabled it
commits image tags back to the repository it watches, which then re-triggers application CI. Path
filters handle that; a separate repo handles it more bluntly. The question belongs with the flux
work (#3424, and the SOPS wiring on #3481), and `.sops.yaml` currently assumes a `gitops/` path
inside this repository.

**The favro-cli source is committed on purpose; only `target/` is excluded.** Two ignore rules
added before the first commit were wrong, and both were verified in an isolated scratch repository
rather than reasoned about:

1. `.agents/skills/.gitignore` containing `favro/` excluded the **whole Favro skill** -
   `SKILL.md` and `references/cli.md`, not just the vendored CLI. That skill is project knowledge
   that has to travel with the repository. The file was removed.
2. Excluding the vendored `favro-cli` source at all would break image reproducibility.
   `Dockerfile.agent` does `COPY .agents/skills/favro/favro-cli` followed by
   `cargo install --path --locked`, so a fresh clone without that source cannot build the agent
   image. `.dockerignore` already draws the correct line - source in, `target` out - and the
   `.gitignore` now matches it. This corrects the agent's own earlier recommendation to keep the
   vendored CLI out of the repository: that advice was given without checking what the image build
   depends on. Committing the vendored copy also preserves the LICENSE-at-repository-root
   requirement recorded on 9 September.

Also fixed: the root `.gitignore` matched `.env` and `*.env` but **not** `.env~`, so an editor
backup holding the real 43-character `TTYD_PASSWORD` was committable. Erik deleted that file; the
pattern gap is closed with `.env*` plus a `!.env.example` exception, so the next editor backup
cannot reintroduce it. With the rules corrected the first commit is 812 KB with nothing over 1 MB,
and a scan of the Markdown, Terraform, TOML, YAML, JSON and shell files found no private-key
blocks, no AWS key ids and no literal token or password assignments. `.sops.yaml` carries only the
age public key and is meant to be committed; `.favro/project.toml` names credentials by environment
variable only.

## favro-cli stays vendored, with provenance pinned — 10 September 2026

Follow-up to the version-control decision above, answering whether the vendored CLI should become
its own repository with a proper remote. It should not, and the reason is structural: FAU never
vendored a repository, it vendored **a subdirectory** of one. Upstream is
`https://github.com/MrGaribaldi/favro-agent-skill`, laid out as `skills/favro/{SKILL.md,
references/, favro-cli/}`, which is exactly what `.agents/skills/favro/` mirrors - so `SKILL.md`
and `references/cli.md` are upstream's files too, not only the crate.

That rules out a git submodule, which is whole-repository-or-nothing. A submodule would have to sit
at a path like `.agents/vendor/favro-agent-skill/`, with `Dockerfile.agent` and the `.dockerignore`
re-include rules re-pointed inside it, and every clone would need `--recurse-submodules` and every
host rebuild an initialised submodule or the `COPY` lands an empty directory. That failure lands on
the one operation the agent cannot drive - the host-side image rebuild - for no gain over vendoring.

Depending on the git URL from the Dockerfile (`cargo install --git … --tag … --locked`) was also
rejected. The skill files must be committed regardless, because agents read them from the workspace
mount and `.dockerignore` never copies them into the image, so the crate would come from one place
and its instructions from another. Split provenance is worse than vendoring both.

**Erik confirmed the upstream account is his own**, so no fork is needed: availability is already
under project control, and a local patch can go upstream rather than into a divergent copy. What was
actually missing was provenance. The vendored tree carried no record of which upstream commit it
came from, so 0.2.1-as-released could not be distinguished from 0.2.1-plus-local-edits, and the only
record was prose in this document.

Recorded in `.agents/skills/favro/UPSTREAM.md`: upstream URL, the pinned commit `e0965df` (Erik's
sha, and both the repository and that commit confirmed reachable 10 September 2026), crate version
0.2.1, the path mapping, sha256 checksums of `LICENSE` and `Cargo.lock` as vendored, and the
re-vendor procedure including the LICENSE byte-identity requirement from the CLI's own README and
the host-side image rebuild that has to follow. The file also states the rule that keeps the pin
meaningful: **nothing in that directory is hand-edited.** A local edit is silently reverted by the
next re-vendor and makes the tree diverge from the pinned sha with no record; FAU-specific Favro
knowledge belongs in `/workspace/CLAUDE.md`, and tool defects go upstream as issues the way #3445
did. Whether the copy still matches `e0965df` is stated as intent rather than verified, because
verifying it means fetching upstream, which happens on the host.

Rebuild behaviour, which was the second half of the question: the vendored source survives all
three senses of "rebuild" unconditionally - a container recreate (the image's
`/usr/local/bin/favro` and the bind-mounted source both persist), a host image rebuild (the source
is always in the build context) and a fresh clone on another machine (`git clone` then
`docker compose build`). A submodule survives the first but breaks the second and third when
uninitialised; a git-URL dependency needs upstream reachable at build time. The already-recorded
trap still stands: a `cargo install` run inside the container does not survive a container
recreate, because `/usr/local/cargo` is not a mounted volume.

