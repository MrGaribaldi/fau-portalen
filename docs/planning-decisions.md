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

## Agent topology: Terraform stays with one agent — 10 September 2026

Erik settled the division of labour for the concurrent agents he is adding. The research and
business agents get **their own container**, which already exists and is not built from agent-box,
so it has none of this box's plumbing - no `/infra-runtime`, no WireGuard, no Docker socket. They
need Favro access and nothing else from here. The remaining agents work on the application side.

**Only this agent works on Terraform.** That is a deliberate single-writer rule, and it removes the
worktree problem raised earlier the same day: `infrastructure/sync-to-runtime.sh` hardcodes
`SRC_ROOT=/workspace/infrastructure`, so infrastructure edits made in a git worktree would never
reach the runtime root, but with infrastructure work confined to the main checkout the script needs
no change. The S3 backend's `use_lockfile = true` remains the backstop rather than the mechanism.

Favro access from a container without `/infra-runtime` is a solved problem and does not need the
credentials file to move: `.favro/project.toml` names `/infra-runtime/infrastructure/favro.env` as
its credentials path, but `FAVRO_ENV_FILE` takes precedence over the configured files - verified on
10 September by pointing it at a deliberately invalid env file and getting a 401 from the API, then
succeeding again without it. So the second container needs only its own env file and that variable
set. The authorship problem is unchanged and gets worse with more agents: every agent posts with
Erik's own token and shows as "Erik W. Bjønnes", so the role emoji prefix is the only signal of who
wrote a comment, and each agent needs its own `--role` key. A separate Favro account for agents was
envisaged in the original coordination decision and would fix attribution at the cost of a seat.

## Placeholder hostname for certificate work: fau-lab.bim.graphics — 10 September 2026

The product name and domain are still open on #3434, and HTTP-01 needs a public hostname that
resolves to the load balancer, so certificate work had no way to start. Erik supplied
`bim.graphics`, a domain he already holds at Domeneshop - the same registrar as the eventual FAU
domain - which serves only a parking notice. Its Microsoft 365 records, an MX, an SPF `-all` and an
`MS=` verification TXT, are leftovers from a product renamed years ago and are not in use.

The placeholder is a **subdomain**, `fau-lab.bim.graphics`, pointing at `lb-fau`'s public v4 and v6
addresses. Using a subdomain rather than the apex was deliberate even after the records turned out
dormant: the record is then purely additive and cannot disturb mail routing or the parked site, and
the reasoning holds if the zone is ever put back into service. Verified before proposing it: the
name is unused, and neither `bim.graphics` nor the `.graphics` TLD carries a CAA record, so Let's
Encrypt may issue.

**Erik creates the record himself in the Domeneshop panel.** No Domeneshop API token enters the
agent container for a placeholder, and none is pasted into a transcript. The judgement was that a
write token for a zone outside the FAU stack is a poor trade for one record created once; when the
real domain exists and its records become infrastructure-as-code, a token in the private runtime
volume is worth it. That is a boundary about blast radius, not about capability.

Two limits recorded with it. The placeholder exercises the issuance path only - nothing a pilot
school sees may live under another product's domain, so #3434 still gates anything user-facing. And
the stale M365 records are best left partly alone: dropping the MX and the `MS=` TXT is tidy, but the
SPF `-all` is the one record there still doing useful work, because it denies spoofing from a domain
nobody is watching.

**A staging ClusterIssuer comes with it.** Upstream's cert-manager module defines only the
production ACME endpoint for both of its issuers. Production allows five duplicate certificates per
week per hostname and a misconfigured challenge loop burns that in minutes, blocking issuance for
days. FAU adds a `letsencrypt-staging` ClusterIssuer in its own root, through the same `raw` chart
technique upstream uses so the issuer is created after cert-manager's CRDs exist - `kubernetes_manifest`
cannot do that at plan time. Inherited and worth knowing rather than chosen: `charts.helm.sh/incubator`
is a deprecated Helm repository that upstream's issuer release already depends on, so both issuer
releases rest on an archived repo.

Sequence, since it caused a false start in planning: ingress-nginx first, because the HTTP-01 solver
hardcodes `ingressClassName = "nginx"` and nothing can validate before the controller exists. Then
the DNS record, then cert-manager with `certificate_email`, issuing against staging before
production. Proposed configuration is quoted in docs/stage-2-remaining-readiness.md and is not
applied; #3485 carries the authorization.

## ingress-nginx applied; the placeholder path works end to end — 10 September 2026

Erik authorized the pass, answered its inputs on #3485 - `certificate_email` is
`kontakt@ewb-solutions.as`, and the empty Cloudflare token is accepted as harmless - created
`fau-lab.bim.graphics` in the Domeneshop panel, and approved the apply. Applied: 2 added, 0 changed,
0 destroyed.

The pass is worth recording for one lesson rather than for its size. **A Terraform plan can
understate what an apply does.** This plan created two Kubernetes objects and no Hetzner resource,
yet the effect was to hand `lb-fau` to the Hetzner CCM, which configured the load balancer's
services and node targets from the controller Service's annotations. Terraform cannot show that,
so "2 to add, 0 to change" was not evidence that nothing would happen to a live, billed resource.
What made it safe was reading stage 1's state first and finding the load balancer **empty** - an
`lb11` with round-robin, no services, no targets, billed since 9 September doing nothing - so the
CCM's work was additive rather than a rewrite. The general rule: when a module's effect runs
through an in-cluster controller, the plan is a statement about Terraform's intentions, not about
the blast radius.

Verified after applying: the controller Service carries the load balancer's v4, v6 and private
addresses on ports 80 and 443; the `nginx` IngressClass exists and is default; both controller pods
run on `master` and `cworker-1`, matching the pre-flight prediction from the nodeSelector
(`cloud=true`, `node-role=worker`) and `vpn-router`'s taint; and nginx answers 404 over HTTP and
HTTP/2 over TLS with its default certificate through the public address, which is the correct state
with no Ingress resources defined. Stage 1 and stage 2 both report `No changes` afterwards - the
CCM's load balancer services do not read as stage-1 drift, because services and targets are
separate resource types stage 1 never declared.

One operational note that will matter at the first certificate attempt: `bim.graphics` has a
negative-answer TTL of 3600 seconds. A resolver that queries a name before it exists caches the
miss for up to an hour, which happened in the agent container. Let's Encrypt resolves from its own
side, so issuance is unaffected, but cert-manager's in-cluster self-check goes through coredns - so
a name queried too early can make a first HTTP-01 attempt look broken when it is only cached.

## Storage defaults and off-node etcd retention — 10 September 2026

Two findings from inspecting the cluster after the CCM and ingress passes (#3488,
docs/cluster-hygiene-findings-2026-09-10.md), both decided the same day.

**StorageClass: the rule, not the re-provision.** The CCM pass left the cluster with **two default
StorageClasses** - `hcloud-volumes` from the `hcloud-csi` release and k3s's packaged `local-path` -
and which one an unqualified PVC receives is version-dependent. Erik chose option (a): every PVC and
every stateful workload sets `storageClassName` explicitly, so the default never decides anything
that matters. The alternatives were rejected on durability and cost: removing the annotation from
`local-path` does not stick, because k3s's addon controller owns that object and reasserts it, and
adding `local-storage` to the k3s `disable:` list would re-trigger the control-plane install through
`config_hash`. The rule is recorded in /workspace/CLAUDE.md so it reaches #3416 and #3424 rather
than being rediscovered when a database is already running. The concrete risk it prevents: a
PostgreSQL volume landing on node-local storage that dies with the node, under a product whose
premise is history that survives.

**Off-node etcd retention: a second flag, not a bug.** Erik chose to check upstream first, and that
settled it. S3 retention is a **separate setting with its own default of 5** - the node's own binary
lists `--etcd-snapshot-retention (default: 5)` and `--etcd-s3-retention (default: 5)` - and FAU's
config sets only the former, to 168. So the observed five hourly snapshots in
`s3://fau-k3s-backup/etcd-backups` against fifteen on disk is the configuration behaving as
written, not a defect. Upstream later added a fallback so an unset `etcd-s3-retention` inherits the
general value (k3s-io/k3s#13770), and the release-1.35 backport of the related regression (#13783)
carries milestone v1.35.3+k3s1; the cluster runs v1.35.2+k3s1, one patch short of the version where
leaving it unset would have been safe.

The fix is to set `etcd-s3-retention: 168` explicitly, which works on the running version and is
preferable to relying on the fallback even after an upgrade, because the value is then declared
rather than inherited. 168 snapshots at ~6.4 MB is about 1.1 GB, a fraction of a cent per month.
Erik's stated preference was to control it in code, so the intended home is a drop-in written by
FAU's own `1-bootstrap` root - k3s reads `/etc/rancher/k3s/config.yaml.d/*.yaml` - with an upstream
request to add the flag to the shared template as the follow-up that lets the divergence retire.
The drop-in restarts k3s on the single control-plane node, so it waits for authorization and a
moment when a brief API outage is acceptable. Scheduling the module's `backup-k3s-server.sh` is no
longer needed for retention, but stays on the table for a different reason: it archives the whole
server directory, which an etcd snapshot does not cover.

## ADR-002 accepted, with a malware scanner added — 10 September 2026

Erik accepted ADR-002 (docs/url-scheme.md, attached to #3446) and answered all three of its open
questions in one comment.

**UUIDv7, and the creation-time leak is accepted.** Time-ordered ids keep index locality and avoid
fragmenting the primary key, at the cost of exposing roughly when a record was created to anyone
holding the id. Erik's judgement: "we can live with leaking creation times". The leak is bounded by
the fact that an id only reaches someone who can already open the resource.

**The unverified-school queue is reviewed manually by Erik, possibly with agent assistance, at
about a week's turnaround.** That is a service level rather than an estimate, and it is what makes
ADR-002's answer coherent: an unverified school is created immediately and works fully at its
`/s/<uuid>` address, staying `noindex` and without a pretty slug, for about a week.

**A malware scanner goes into the document ingest pipeline.** This is new scope rather than a
confirmation - ADR-002 had left it as an open item deliberately. Erik's reason widens it beyond
school submissions: FAU-er will upload their own existing document archives, which is exactly the
material most likely to carry something old and infected, arriving from people who are not
attackers and cannot be expected to vet it.

The consequence to settle before anything is built is a supplier question, not a routing one.
**An external scanning API would send member documents out of the cluster**, which makes the
scanner a customer-facing supplier under the ownership policy of 9 September: European ownership
required, and a new entry on #3409. A scanner running in-cluster - ClamAV being the obvious
candidate - keeps documents inside the boundary and needs no supplier decision at all. The
in-cluster route is not free either: a ClamAV daemon holds its signature database resident, which
belongs in the sizing on #3408 rather than being discovered when a node starts swapping. Tracked
on #3447 alongside the active-content stripping it does not replace - scanning and sanitisation
answer different threats, and ADR-002 keeps them separate on purpose.

## S3 credential rotation scheduled — 10 September 2026

Erik decided to rotate the leaked S3 credentials tomorrow rather than immediately, on the reasoning
that nothing indicates the transcript has been read by anyone else and the rotation touches five
places including a control-plane decision. Recorded so the delay is a choice with a date rather
than a thing that quietly does not happen. Procedure and the open question about step 4 - whether
to let stage 1 re-run the control-plane install or patch `master` with a targeted drop-in and accept
pending `config_hash` drift - are on #3489.

## Malware scanning in-house, on a dedicated ingest worker — 10 September 2026

Erik decided the scanner runs **in-house** rather than through an external scanning API, which
closes the supplier question the ADR-002 acceptance opened: no member document leaves the cluster,
so the scanner needs no entry on #3409 and no European-ownership decision. His proposed shape is a
worker spun up when a user uploads files, doing both the scanning and the conversion of other
formats into FAU's Markdown.

**The dedicated worker is worth having, and the strongest argument for it is isolation rather than
capacity.** Conversion means running LibreOffice, PDF tooling and image libraries over files
supplied by strangers - the most CVE-dense components in any ingest pipeline. A worker holding no
database credentials, reading only from the quarantine bucket and writing only results, means the
parser can be compromised without reaching the API, the database or the servable bucket. Memory is
the second argument: ClamAV keeps its signature database resident, on the order of a gigabyte before
it scans anything, against `cx23` nodes with 4 GB that will also run the API and PostgreSQL.

**Two corrections from querying the Hetzner API rather than the catalogue.** `cpx33` does not
exist - the line is cpx11/12, 21/22, 31/32, 41/42, 51/52, 62 - and the nearest type to the intent is
`cpx32`, 4 vCPU and 8 GB at EUR 35.49/month or EUR 0.0569/hour in hel1. More importantly, Erik's
availability instinct was sharper than expected: in `hel1-dc2` on 10 September only the "2"
generation of cpx is in stock, and **`cx33` cannot be created at all** - the type an earlier draft
of the assessment had recommended running always-on at EUR 8.49. That is the same flicker Erik
watched in the cx line on 8 and 9 September, and it becomes a design rule: **a node pool names an
ordered list of acceptable types, not one type**, and provisioning checks availability rather than
trusting the catalogue.

**The economics therefore favour the on-demand shape.** An always-on `cpx32` at EUR 35.49 would more
than double the current EUR 25.46 infrastructure bill; the same worker used an hour a day costs about
EUR 1.70, and the monthly cap is only reached at roughly 623 hours. The recommendation recorded on
#3447 is nonetheless to build the pipeline before the node: upload to a quarantine bucket, a queue
entry, a Kubernetes Job that scans and converts, results written back, the API only reading status.
That asynchrony is forced by provisioning latency anyway - server creation plus cloud-init plus k3s
join is minutes, and nobody uploading a PDF waits - and once the pipeline is asynchronous the node
can be always-on, on-demand or replaced later without touching the application. Upstream ships no
cluster-autoscaler, so the on-demand machinery is real work with quiet failure modes: a node that
fails to join is a billed server outside Terraform state, and a scale-down mid-scan loses the job.

**Conversion to Markdown carries a product risk that is not an infrastructure question.** DOCX, ODT
and PDF conversion is lossy exactly where FAU's documents matter - tables, footnotes, annexes - and
these are minutes and records. The rule recorded to keep that safe, consistent with #3412 and
ADR-002: the uploaded original is the authoritative artifact, stored immutably, and the Markdown is
a derived working copy that can be regenerated when the converter improves. Converter choice and
fidelity expectations belong with the Markdown and text-import work on #3419.

Scanning does not replace the active-content stripping already specified on #3447: a stripped file
can still be malware and a clean-scanning file can still carry active content, so both stay in the
pipeline. The scanner's signature updates are the one new outbound dependency the worker adds.

## Ingest worker: cpx32, created for the work and deleted after — 10 September 2026

Erik's decision on the shape, following the in-house scanning decision the same day: a `cpx32`
(4 vCPU, 8 GB, EUR 0.0569/hour in hel1, EUR 35.49/month if left running) is created to scan and
convert an upload, then shut down, so the cost tracks actual use instead of an idle node 99% of the
time. `cpx32` also answers the availability point: only the "2" generation of cpx is in stock in
hel1, and `cx33` cannot be created there at all today.

Four mechanics decide whether the saving is real, and they are recorded because each is a way the
design quietly fails to save anything:

1. **"Shut down" must mean delete.** Hetzner bills a server while it exists, powered off included.
   So the lifecycle is create and destroy, which in turn means the worker holds no state worth
   keeping. To be confirmed against the first invoice rather than assumed.
2. **Assume the hour is the billing unit and batch accordingly.** A node takes minutes to create,
   join and warm up. Two uploads ten minutes apart should share one worker, so the queue drains in
   windows: a node comes up when work is waiting, drains, and is deleted after a short idle timeout.
   One node per uploaded file would be both slow and, if partial hours round up, no cheaper.
3. **No public IPv4 on the worker.** A primary IPv4 costs EUR 0.50/month and keeps billing if it
   outlives the server. It is also unnecessary - the private network already routes `0.0.0.0/0` to
   the vpn-router at `10.0.1.254` - and the security argument is stronger than the saving: the
   machine that parses hostile files should not be reachable from the internet.
4. **Stale Node cleanup is already handled** by the Hetzner CCM applied earlier the same day, which
   removes Node objects whose servers no longer exist. A direct benefit of that pass.

The mechanism is the Kubernetes cluster-autoscaler with the hcloud provider and a node pool that
scales to zero: a pending scan Job brings a node up, an idle timeout takes it away. Infra-tools
ships no autoscaler, so this is new work in FAU's own root, with an hcloud token secret in the
cluster. The failure modes to test deliberately are a node that fails to join, which leaves a billed
server outside Terraform state, and a scale-down during a scan, which loses the job unless the Job
blocks eviction. The pool names `cpx32` first and `cx23` behind it, so a stock shortage degrades to a
slower worker rather than a failed upload.

The order is unchanged: the pipeline is built first on the existing cluster - quarantine bucket,
queue entry, Kubernetes Job, results written back, API reads status only - because provisioning
latency forces asynchrony regardless, and an asynchronous pipeline lets the node be ephemeral,
permanent or a different type without touching the application. The invoice after the first month
decides whether ephemeral was the right call: if uploads arrive all day the monthly cap applies and
one always-on node is cheaper and simpler.

## Ingest tiers and the quarantine rule — 10 September 2026

Erik refined the ingest design the same afternoon, and the result is simpler than the per-batch node
the assessment proposed. Four tiers, in order of what handles a given upload:

1. **Single files scan inline on an existing node**, paying the cost in RAM.
2. **Bulk uploads are processed once a day**, as one batch.
3. **If that node is fast enough, it does the bulk batch as well.**
4. **If the queue is large enough, a `cpx32` is created for the work and killed afterwards.**

The ephemeral node therefore becomes an escalation rather than the default, which takes provisioning
latency out of the common case entirely: a single upload never waits for a server to be created.

**The RAM price, stated rather than waved at.** "An existing node" means `cworker-1` - `master` runs
the control plane and etcd, `vpn-router` is tainted - and it is a `cx23` with 4 GB that will also
host the API and PostgreSQL. A resident `clamd` holds roughly a gigabyte of signatures before it
scans anything, which is the whole margin. Two cheaper ways to the same guarantee are recorded on
#3447: `clamscan` per file, trading a permanent reservation for a 1 GB spike and several seconds of
database loading per scan, which suits occasional single files and is clearly wrong for a bulk
batch; or a second `cx23` at EUR 5.49/month dedicated to ingest at concurrency one, which is cheaper
than an idle `cpx32` and keeps the parser off the node holding the database. The decision is to
measure first - scan timings and what they do to `cworker-1`'s memory - and let that choose. The
escalation threshold should likewise be expressed in estimated work rather than file count, since
provisioning costs minutes; a first cut is fifteen minutes of estimated scanning, revised on real
timings.

**The scan has two outcomes: cleared, or quarantined.** Cleared files are converted to Markdown
keeping formatting. For quarantined files Erik's rule is to check whether the problem is a macro or
something else easily disabled, convert the text if so, and otherwise tell the user the file is
infected and cannot be processed.

The second half needed a safe formulation, because **a malware detection is not a description of
what is wrong**: ClamAV returns a signature name, not "there is a macro you could remove", so
deciding "this is only a macro" from that name would be guesswork on exactly the input where
guessing is expensive. The formulation recorded instead does not ask the scanner what is wrong; it
asks **whether the threat can survive our conversion**. A macro cannot cross the boundary into
Markdown, so a document whose only malicious element is a macro is made harmless by the conversion
itself. What does not survive that reasoning is the case where the conversion *is* the attack - a
malformed PDF, image or font crafted against the parser that opens it. So the rule is file-type and
category based, conservative by default:

- Macro or script in an office document, including JavaScript in a PDF: convert **in isolation**, on
  the ephemeral worker, never on the node holding the database; the original stays quarantined and
  is never served.
- Detection where the risk is the parser itself - malformed PDF, image, font: **refuse** and tell the
  user.
- Executables, scripts, or archives containing them: **refuse**; nothing in FAU's document model
  needs them.
- Anything we cannot categorise: **refuse** and log for review. The default is refuse, not convert.

Two consequences fixed with it. The quarantine-retry path is the strongest argument yet for the
ephemeral worker, because converting a file known to be infected is exactly the work that belongs on
a machine with no database credentials, no public address and a short life. And both the user-facing
message and the audit record matter: a refusal should say what to do next rather than only reporting
failure, since a member uploading a ten-year-old archive is not the attacker and may have no clean
copy; and a file converted despite a detection must leave that decision in the audit trail, because
someone later asking why a document differs from its original needs an answer. Retention of
quarantined originals belongs with #3426.

**"Keeping formatting" has written limits.** Representable and expected to survive: headings, bold
and italic, lists, links, footnotes, block quotes, code blocks and simple tables. Lost or
approximated: page layout, columns, text boxes, fonts, tracked changes, comments, merged-cell
tables, embedded spreadsheets and drawings. The rule that keeps this honest is the one already
recorded - the uploaded original stays the authoritative artifact and the Markdown is a derived
working copy - so a lost merged cell is a display limitation rather than a lost record. A conversion
that meets something it cannot represent should say so on the document rather than dropping it
silently.


## Structured rich-text direction — 11 September 2026

Erik authorized applying the coordinated rich-text Favro review. Schema-constrained
Tiptap/ProseMirror JSON replaces Markdown as canonical editable content. Tiptap is
preferred pending #3415 integration, accessibility and licence evidence; the frontend
framework remains unselected. HTML is a sanitized derivative. Physical persistence
must preserve #3484 encryption, including comments and private rendering caches;
plaintext JSONB is not an approved shortcut.

Preserve session-grouped full snapshots, durable autosaves, audit atomicity and
#3420 exclusive leases/stale-write rejection. Originals remain unchanged private
import provenance subject to quarantine and retention; JSON governs later edits.

New deliverables: #3490 architecture/schema/Rust-compatible rendering amendment,
#3491 private anchored comments, #3492 controlled publication snapshots. #3412
remains Done with historical approval intact. Publication structurally excludes
comments, editorial metadata and private blocks and releases only authorized assets.
Define anchor mapping/orphans and imported authors separately from account identity.

Updated cards: #3419, #3412, #3415, #3422, #3447, #3435, #3484, #3426,
#3409, #3428, #3433 and #3425. Existing titles are retained. DOCX refusal is
superseded when a validated DOCX import path is scheduled; other Office formats
remain refused absent separate specification. Malware scanning remains required
alongside active-content stripping. Richer conversion needs a new safety assessment;
JSON alone is no safety guarantee. Converter-specific fidelity must be tested.

DOCX/original retention, comments and publication remain separate scope decisions
on #3433. Simultaneous editing/Yjs and commercial high-fidelity conversion remain
deferred. No deployment, subscription or external data transfer is approved.
Extensive SSD backup work remains deferred; eventual restore coverage is extended.
Earlier Markdown-only assumptions are superseded by this direction.

## Frontend preference and separate editor decision — 11 September 2026

Erik confirmed Rust-rendered HTML with htmx and focused JavaScript/TypeScript as
the preferred frontend direction. The editor remains an embedded interactive
component; an application-wide component framework is not the preferred default.

Erik requested that major decisions have a separate Favro card with an attachment
comparing alternatives, pros and cons for each, and a recommendation at the bottom.
This convention is preserved in AGENTS.md. Editor decision #3493 now owns review
of docs/editor-alternatives.md, also attached to the card. #3415 links to it.
The earlier Tiptap preference is reopened for comparison; no editor package or
paid platform is selected by this update. The proposed shortlist and recommendation
are research for review, not approval. Existing encrypted persistence and exclusive
editing requirements remain inputs to the evaluation; a different document model
requires explicit coordination with #3490.

Erik subsequently identified Lexical as the leading candidate because its playground
matches the desired editing experience. This preference supersedes the report's
provisional ordering, not the requirement for integration evidence. Avoiding React
and a Node production backend is motivated by lower security maintenance. The
playground uses React; equivalent non-React feature coverage and porting effort
remain to be established. Statically served editor JavaScript still needs updates.

Erik then accepted compiled browser-only React as a possible editor integration:
mostly Rust-rendered HTML/htmx, with a React/Lexical editor served as static assets
by Rust. This qualifies the earlier no-React preference; no Node production backend
or React Server Components are proposed. Evaluate desired playground feature reuse
against vanilla-JavaScript integration effort on #3493 before final selection.

## Live decision reconciliation — 11 September 2026

Erik subsequently chose Lexical for the editor for now because its playground
matches the desired editing experience, and accepted exploring compiled React for
the editor. #3493 is therefore Done as the editor selection decision, with the
attached alternatives comparison retained as its evidence. This is a provisional
package choice pending the bounded integration and fidelity checks; it does not
approve a paid Tiptap platform, a Node production backend, or React Server
Components.

#3415 is Done as the frontend direction decision: Rust-rendered HTML with htmx and
focused JavaScript/TypeScript, with an isolated compiled browser-only
React/Lexical editor served as static assets by Rust. The rest of the application stays HTML/htmx; React is scoped to the editor page.

Remaining work is handed to #3490 and #3422. #3490 must explicitly compare the
Lexical document model, schema validation and Rust rendering contract with the
older Tiptap/ProseMirror assumption while retaining encryption, snapshots,
autosave, audit atomicity and exclusive leases. #3422 must validate static asset
serving, editor lifecycle around htmx, unsaved-change handling, accessibility,
bundle/runtime impact and licence/package evidence. The completed decision cards
retain their original comparison and accessibility/licence requirements; these
checks are implementation acceptance work, not silently completed by the
decision.

## Conversion target is HTML, and the storage-format question it opens — 11 September 2026

Erik proposed converting DOCX to **HTML** rather than Markdown, on two grounds: HTML imports easily
into the editor, and the conversion itself may strip exploits. Both hold, the second conditionally.

**HTML is the import intermediate.** The selected editor is Lexical in a React page;
the rest of the app stays Rust-rendered HTML/htmx. The earlier Tiptap/ProseMirror
assumption is superseded. Conversion must map sanitized HTML into the bounded
Lexical profile defined through #3490 and report unsupported formatting.

**Import and storage formats are distinct.** Schema-constrained structured JSON
is canonical editable content, encrypted under #3484. HTML is a sanitized
import/rendering derivative; Markdown-only storage is superseded. The exact
Lexical wire profile, validation, snapshots and Rust renderer are specified in
docs/structured-document-architecture.md and validated through #3490/#3422.

**The security claim depends on the converter's architecture, and the two kinds are opposites.**
Parse-and-rebuild converters - `mammoth` being the clearest - read the document model and emit HTML
from a small set of elements they understand, driven by an explicit style map. Macros, OLE objects,
embedded binaries, field codes and remote references are dropped because the converter has no code
to emit them: sanitisation by construction, which is what makes Erik's point true. Render-and-export
converters - LibreOffice `--convert-to html` - use the same full document engine an attacker targets
and then serialise the result; the output is cleaner than the input but the process is the exposure,
and the HTML is messy.

The framing recorded to keep this honest: **conversion sanitises the output, never the input.** The
parser still eats hostile bytes either way, which is precisely why the isolation boundary and the
ephemeral worker remain necessary.

Three hard requirements follow from DOCX itself:

1. **The converter fetches nothing.** A DOCX can carry a remote template reference in
   `settings.xml`, external relationship targets, `INCLUDEPICTURE` and DDE field codes; resolving
   any of them turns an upload into an outbound request from inside our network, which is
   server-side request forgery and, against SMB-style targets, a credential leak. Enforced rather
   than trusted: the conversion Job runs under a NetworkPolicy denying egress. Since ClamAV needs
   egress for signature updates, those are separate Jobs with separate policies.
2. **Converter output is still untrusted** and passes an allowlist sanitiser before storage or
   editor import - no script, event handlers, `javascript:`/`data:` URLs, iframes, objects or
   embeds, and SVG refused or sanitised as its own format because it is script-capable. This is
   #3447's active-content stripping, now applied to the converted HTML.
3. **Images are re-encoded, never passed through**, because the reader's browser image parser is
   where exploit risk lands; anything that fails to decode is dropped.

Plus zip-level hygiene, since DOCX is a zip: entry-count and uncompressed-size caps against zip
bombs, rejection of absolute or traversing paths, and refusal of encrypted documents, which cannot
be scanned or converted meaningfully.

Recommendation recorded on #3447: `mammoth` as the default DOCX converter, chosen for the
parse-and-rebuild property rather than for fidelity, with an explicit reviewed style map;
LibreOffice headless only as an opt-in fidelity fallback inside the isolated worker with egress
denied. PDF is deliberately not folded in by analogy - PDF-to-HTML is lossy where records matter and
PDF parsers are the CVE-heavy end of the field, so PDFs stay attachments with text extraction for
search rather than becoming editable content.


## Editor page boundary clarification — 11 September 2026

The editor uses Lexical (lexical.dev) in an isolated React page. The rest of the application remains Rust-rendered HTML/HTMX with focused JavaScript/TypeScript. Rust serves the compiled browser assets. The earlier Tiptap/ProseMirror preference is superseded; editor integration and the exact versioned JSON profile still require validation on #3422/#3490.

This reconciles the existing decisions on #3415/#3493, rather than reopening the frontend selection. HTML import must map into the validated Lexical profile; ProseMirror JSON is historical comparison material, not the selected storage format. Existing encryption, snapshots, audit, autosave, exclusive leases and scope decisions remain in force.

## Identity provider changed to PropelAuth — 21 September 2026

Erik replaced Zitadel Cloud with PropelAuth for authentication, accepting a US supplier for a
customer-facing service. The reasoning he gave: PropelAuth is already in production on another
EWB service, so it is known to work rather than merely evaluated; and the provider will hold only
user email addresses and organisation membership, never document content.

This is a deliberate exception to the European ownership rule rather than a case that satisfies
it. Recorded plainly because the rule exists to stop unexamined drift, and an exception that is
argued is not the same as one that is unnoticed. PropelAuth's primary processing is in the United
States, it offers no EU data residency option, its DPA applies EU Standard Contractual Clauses
(Module Two) under Irish law, and it names fourteen subprocessors, all US-based - AWS for hosting
and Postmark for mail among them, with ten days' notice on changes.

What the change supersedes: the Zitadel Cloud Swiss-region choice, the "time-boxed exception"
framing around it, the open question about Zitadel's free-tier metric, and the search for a
European replacement as a precondition for proceeding. What it does not change: the authorization
boundary - the provider authenticates, our database authorizes, always - and the portability
rules in ADR-003 decision 3, which now matter more rather than less, since they are no longer
backed by an intention to leave.

Commercially the change is favourable. PropelAuth's Free tier covers 10,000 monthly active users
with unlimited organisations, which is above any realistic MVP or pilot scale, so the pricing
metric no longer constrains session design the way Zitadel's daily-active-user counting did.
Free includes a test environment; a separate staging environment requires the Growth plan at
USD 150/month.

Two questions the change opens are on #3484 rather than settled here: exactly what PropelAuth is
allowed to hold, since mirroring FAU membership into its native organisations means a US
processor knows that a named parent is attached to a named school; and who sends authentication
email, since PropelAuth's hosted flow sends it through Postmark and thereby moves one
customer-facing mail path outside the European email decision on #3410.

## Administrative two-factor: TOTP at login plus a freshness gate — 21 September 2026

Erik answered #3414. Privileged administrative actions are gated on two server-side checks rather
than on true step-up MFA: the account must have TOTP enrolled, read from PropelAuth's
`mfaEnabled` flag, and the session must be fresh, meaning a login within the last 30-60 minutes.
A stale session is forced back through login, which re-triggers TOTP. PropelAuth's real step-up
MFA requires the Growth plan at USD 150/month and was not judged worth the cost.

Erik stated the limitation himself: this is not per-action step-up. There is no challenge bound
to the specific operation and no action-scoped one-time grant, so it does not defend against an
attacker already inside a fresh, MFA'd session. Accepted for this threat model - volunteer
administrators on shared or forgotten devices - rather than enterprise-grade replay protection.

This answers Codex's review finding of 10 September, which was correct that the earlier
formulation did not establish two factors: repeating the email factor is one factor twice. TOTP
is a genuinely independent second factor, so prosjektgrunnlag.md section 13's requirement for
administrative roles is satisfied at login, and ordinary parents reading minutes are not asked
for one.

One correction to the plan as Erik wrote it on #3414. Its fourth step - enforcing "require MFA"
at organisation level through a PropelAuth setting - is not available on the Free plan, where
org-level 2FA enforcement is a Growth Plus feature. It is also unnecessary: the `mfaEnabled`
check enforces the same property server-side, on our side of the authorization boundary, which is
where ADR-003 decision 2 puts it anyway. TOTP itself is available to users on Free.

Still to define before the guard ships: the explicit list of sensitive endpoints it covers, and
the freshness threshold within the 30-60 minute range.

## What PropelAuth is given, and who sends the login mail — 21 September 2026

Erik answered the two questions the provider change opened on #3484.

**Data minimisation, stated as a rule rather than a preference.** PropelAuth gets no more
information than it needs, and specifically no names. It holds an email address and which
organisations an account belongs to. It does not hold names, other user properties, roles,
permissions, document content, titles or filenames. FAU membership is mirrored into PropelAuth
organisations for login context only; every authorization decision is taken against our own
database, per operation, which is the rule that keeps the provider replaceable.

Two consequences survive minimisation and are recorded so they are not rediscovered later. A US
processor still learns that an account is attached to a particular organisation, which is personal
data whenever the organisation identifies a school. That makes organisation naming a privacy
decision: PropelAuth organisations should carry our opaque tenant identifier rather than a
readable school name, since our own application renders FAU switching and the hosted login page
has no need for the name.

**Login email goes through PropelAuth for now.** Its hosted flow sends authentication mail
through Postmark, in the US. Accepted deliberately, with a stated trigger for revisiting it:
move authentication mail to a European provider when the product is popular enough to fund the
work. So the European transactional-mail decision on #3410 covers every customer-facing message
except this one, and the gap is dated rather than overlooked. Recorded with its trigger because
"for now" decisions that carry no condition tend to become permanent.

**Clarification to the administrative-MFA decision.** Erik's fourth step on #3414 - "enforce
org-level require MFA" - means our own server-side enforcement, checking TOTP enrolment and the
age of the last MFA-backed login. It does not mean PropelAuth's organisation-level "require 2FA"
setting, which is a Growth Plus feature unavailable on Free. Erik agreed the wording should not
point at the provider setting. The better reason not to use it is that enforcement belongs on our
side of the authorization boundary, where ADR-003 decision 2 puts every authorization decision.

## Identity provider changed again, to Hanko — 22 September 2026

PropelAuth is dropped one day after being chosen. The reason is disqualifying rather than a
matter of preference: **it does not offer a DPA on the Free tier**, which is the tier the decision
assumed. A processor handling personal data on our behalf without an Article 28 agreement is not
a supplier we can lawfully use, whatever else it does well. That the gap surfaced through #3484's
question 1 - "approve signing PropelAuth's DPA" - is the case for asking the paperwork question
early rather than at contract time.

The replacement is **Hanko** (Hanko GmbH, Kiel, Germany), found by Erik. It offers a Hanko Cloud
DPA, operates under GDPR as its default rather than as a bolt-on, and states that ISO 27001 is in
progress - recorded as Hanko's claim, not independently verified. Free to 10,000 monthly active
users, then USD 0.01 per MAU, with a startup programme offering 1M MAU free that is worth
applying for.

**This closes more than it opens, and it is worth being explicit about what falls away.**

- The European ownership rule is now **satisfied rather than excepted**. FAU no longer carries a
  customer-facing exception for identity, and the exception paragraph added to CLAUDE.md on
  21 September is reduced accordingly.
- **Hanko has no organisation model.** Its multi-tenancy isolates whole user pools per deployment
  rather than grouping members inside one, so there is no FAU membership to mirror. The provider
  holds an email address and the authentication material the user creates, and nothing else. The
  21 September minimisation rule now holds structurally instead of by discipline, and the question
  of whether to give organisations opaque names is moot.
- **Authentication mail no longer leaves the EU.** The 21 September decision to accept US-hosted
  login mail through Postmark "for now, until revenue allows" is superseded and simply unnecessary.
- **The exit stops being theoretical.** Hanko is open source and self-hostable, so ADR-003
  decision 3's portability rules point at a route we could actually take rather than at a
  destination that had not been found.
- **The emailed-code question answers itself.** Hanko's native mechanism is a six-digit email
  passcode, which is what ADR-003 section 4a argued for on its own merits - school and municipal
  mail scanners consume single-use magic links before the recipient clicks them.

**The qualification Erik raised, recorded as he framed it.** Hanko's infrastructure subprocessors
include AWS, which is US-owned even when the region is in the EU, so the chain still reaches a US
company. He accepted it on the grounds that Hanko documents the arrangement, and that it is "not
a problem right now". The distinction that makes this different from PropelAuth is worth keeping
precise rather than glossing: the controller is German, the data stays in the EU, and there is no
transfer to the United States needing a legal basis. Hanko also lists Hetzner and adesso as a
service as infrastructure, so AWS may not be the only path. Revisit if Hanko's hosting changes, or
if US access to EU-resident data becomes a live issue rather than a theoretical one.

**What survived the change untouched**, which is the useful signal: the authorization boundary
(the provider authenticates, our database authorizes, per operation), the portability rules, and
the administrative-MFA design. The MFA design maps directly onto Hanko, whose user object exposes
`totp_enabled`, `auth_app_set_up` and `security_keys_enabled` - better than the single flag the
design was written against, because it distinguishes a TOTP app from a FIDO security key. The
reasoning for enforcing MFA server-side rather than through a provider setting was written for a
different provider and needed no edit at all.

One new question the change opens, now #3484's open item 3: Hanko is passkey-first, and a passkey
is a stronger factor than a mailed code, but our users are parents on whatever device they own and
a passkey living on one phone becomes a support call when that phone is replaced. Whether passkeys
ship in the MVP alongside the passcode baseline is undecided. A related point for the admin gate:
a passkey login already proves possession plus a local biometric or PIN, so it should satisfy the
second-factor check rather than having TOTP demanded on top of it.

## Deletion made provable: per-FAU key-encryption keys, and a key replica with queued deletion — 22 September 2026

Erik accepted softening ADR-003 section 7's deletion claim now and implementing the design that
makes it true, and added the requirement that produced the interesting part.

**The flaw, and why it was a design problem rather than a wording problem.** Section 7 claimed
that destroying an FAU's wrapped data key made its content unrecoverable "including in every
backup already written". Decision 5 stored that wrapped key on the tenant row - inside the
database, therefore inside every database backup - while the master key that unwraps it stayed
alive in the key service. Restore last night's dump and the content returns. Codex's review of
10 September found this; it was correct.

**The fix is one tier.** The wrapping key becomes **per FAU** and lives only in the key service's
own datastore, never in the application database. Destruction targets that key. A restored
database backup then yields a wrapped data key that nothing can unwrap, because the key that
unwraps it was never in the backup. This is ordinary crypto-shredding, and it costs an extra tier
in a hierarchy the design already required.

It also buys something the row-stored design could not: content ciphertext can sit in versioned or
immutable storage indefinitely without defeating deletion, so the rest of the backup design gets
to follow ordinary hygiene. Only the key store carries the special requirement.

**Erik's requirement: the keys need a backup.** If ransomware reaches the key service and there is
no key backup, every FAU's data is permanently gone - a worse failure than the one crypto-shredding
defends against. So the KEK store is replicated. His resolution of the tension that creates is the
design's centre: **the service queues a deletion against the replica that runs after a few hours.**

Expanded into a shape that can be built:

- The replica holds per-FAU key records, individually addressable, in a store with real deletes.
  It is explicitly **not a generational snapshot history** - deleting from today's snapshot does
  nothing about last week's - so there are no periodic snapshots of key material, only one live
  replica that deletion propagates into.
- The replica's records are encrypted under a **backup root key that is not on the cluster and not
  in the replica**, so possession of the replica alone yields nothing. Without this the replica is
  simply a second copy of the crown jewels, and durability would have been bought by doubling the
  attack surface.
- The replica uses different credentials from the key service, so one compromise does not reach
  both, and the queue is executed by the replica side rather than driven from the live service -
  otherwise compromising the key service also grants the power to cancel.
- Destruction removes the key from the live service immediately and enqueues removal from the
  replica after the window. During the window it can be cancelled, which is what saves us from an
  accidental or unauthorised destruction.
- **A queued deletion is a critical alert, not a log line.** An attacker who enqueues deletion for
  every FAU and simply waits out the window defeats the design in silence. So the queue pages a
  human through #3442's critical path, bulk enqueueing is rate-limited exactly as bulk unwrapping
  is, and cancellation is itself audited and alerted - "the deletion you queued was cancelled" is
  precisely what a successful attacker wants to happen quietly.

**A second hiding place closed while writing this.** Key material must not live in a Kubernetes
Secret either: a Secret lives in etcd, etcd is snapshotted to `fau-k3s-backup`, and a key destroyed
in the live cluster would survive in every snapshot. The same hole one layer down. The key service
needs its own datastore.

**The claim now reads so it is provable.** Destroying an FAU's key-encryption key makes its content
unrecoverable, in the live system and in every database backup, once the queued destruction has
completed; until then it is recoverable and the recovery is audited. A cryptographic fact with a
bounded window rather than an unbounded promise. The rehearsal extends #3425 rather than inventing
a separate exercise, and includes the negative control - a restore for an FAU whose key was never
destroyed must succeed, or a failed decryption proves nothing but a broken test.

Two parameters remain open on #3484: the length of the window, where twelve hours is proposed on
the argument that it must survive a night, and the custody of the backup root key, which points at
#3481 and an offline password manager.

## Deletion timings: freeze on request, confirm by role, destroy after seven days — 22 September 2026

Erik set the deletion timings, and his answer describes two timers rather than one. They are kept
separate in ADR-003 because they defend against different things.

**The replica window is now seven days**, up from the twelve hours proposed. Erik's instruction was
at least 48 hours and probably 72 or more, on the reasoning that a destruction scheduled late on a
Friday evening must not complete before anyone is back at work. That argument is better than the
overnight one it replaced. Seven days rather than 72 hours because 72 hours does not survive a
Norwegian Easter: Skjærtorsdag through 2. påskedag is five days, so a destruction enqueued on the
Wednesday evening would complete untouched. Seven days covers every weekend and holiday cluster in
the school year with one number and no calendar logic, and lengthening it costs nothing but how
long deletion takes to become final.

**The delete request freezes the FAU immediately**, before any confirmation. Billing stops,
invitations stop, and the FAU goes read-only - but **reads continue**, deliberately, because the
freeze is exactly when members should be exporting what they need and a deletion flow that removes
access before it removes data is hostile. The freeze is what makes a long wait cheap: the FAU is
costing us nothing and costing them nothing irreversible, so there is no pressure to hurry.

**Who confirms depends on what the FAU has been, not on who is asking.** An FAU that has only ever
had one member may be deleted by that member alone - no shared history, nobody else harmed. Every
other FAU needs confirmation, because the last remaining member of a council that once had ten is
not entitled to destroy nine other people's record of it. The test is on membership *history*
rather than current count, which is answerable precisely because #3412's approved model keeps
date-ranged roles rather than a current-members list. Where the recovery contact is a school
representative, confirmation is requested from them and we wait at least seven days; silence is
not consent, and an unanswered request stays frozen and pending rather than proceeding by default.

**School holidays extend the wait rather than defeating it.** An FAU is dormant through the summer,
and a confirmation sent in mid-July may reach nobody until mid-August, which would make a
seven-day clock a formality. The rule proposed: if the wait would expire inside a defined
school-holiday period, the clock restarts at the end of it. Which periods count is still to be
named - summer is the one that matters, and whether Christmas and Easter are included is a
judgement rather than a fact.

**A distinction recorded before it causes a compliance error.** Two different things are being
called deletion. Closing an FAU account is a tenant-lifecycle event where a wind-down measured in
weeks is normal. An individual exercising erasure under GDPR Article 17 is a different request,
with a one-month statutory response time, and it concerns that person's own personal data - their
address, their membership record - not the FAU's shared documents, which they neither own nor may
unilaterally destroy. The flow above is the first. The second belongs on #3426 and must not
inherit these waits.

## ADR-003 closed out: key lifetime, search, retention and the remaining timings — 22 September 2026

Erik answered the last questions on #3484. Two of his answers changed the architecture rather than
merely selecting from what was offered.

**Key lifetime: a session-scoped handle, not a fetch per operation.** This reverses the
recommendation the ADR carried, and Erik's question is what exposed it. His scenario: someone
editing minutes during a meeting writes a few lines, talks for several minutes, writes a few more.
Autosaves are durable and frequent by requirement, so per-operation key fetching means a key-service
round trip every few minutes per editor, for hours, across every FAU meeting on the same evening.

The decisive argument turned out not to be performance. Decision 5 exists so that mass decryption
looks like mass decryption in the logs; a log carrying one unwrap per session per FAU makes an
unusual pattern obvious, while a log carrying one unwrap per autosave buries two hundred malicious
entries inside tens of thousands of routine ones. Per-operation fetching dilutes the very signal
the key service was built to produce. Nor does it buy protection: a compromised backend holds
plaintext by construction - it must, to serve a document to its reader - and can ask per operation
as easily as once. The controls that matter are the rate and the ceiling, not the interval.

So the backend obtains a key once per user session per FAU and holds it in memory, with an idle
timeout in the tens of minutes, refreshed by real user activity only - no background timer, no
keep-alive on an idle tab, the same rule already applied to token refresh and for the same reason.
Zeroised on logout, session end, idle expiry and lease release. Never on disk, never logged, never
in a crash dump; swap disabled and core dumps off, since a key reaching disk by accident is the
same class of mistake as putting it in a Kubernetes Secret. Replacing per-operation logging as the
control, the key service gains a **concurrency ceiling** on how many distinct FAU keys one backend
instance may hold at once, plus an alert on the rate of new acquisitions - a wall rather than a log
entry.

**Notifying an idle viewer needs no decryption**, which was the other half of Erik's question.
Editing is exclusive by #3420, so the second person is a viewer or is waiting for the lease. Both
need to know the document changed, and neither needs plaintext to be told: the notification carries
a document identifier and a revision number, and both are already plaintext by decision 6. Only the
subsequent fetch touches ciphertext, and by then the viewer is acting, which re-establishes the key
handle under the activity rule. In practice a small server-sent-events stream per FAU, which htmx
consumes natively and which therefore needs no React outside the editor page.

Recorded alongside it, because Erik's question implied it and the document had never said it
plainly: **encryption is server-side**. The React editor never holds an FAU key; it sends document
JSON to the backend over TLS and the backend encrypts before storage. This is not end-to-end
encryption, it follows from the trust model, and it is why "we cannot see your data" stays
forbidden.

**Search is filename search, and nothing leaves the encryption boundary.** Erik was explicit that
we do not move data out of encryption to make a feature easier, and asked whether an index would be
better than decrypting on demand. Decrypt-on-demand wins at this scale and it is not close: an FAU
with 500 documents holds perhaps 30 KB of filename ciphertext, decrypted in under a millisecond
inside a session whose key is already unwrapped, against an index that must stay consistent with
every rename and deletion. An index is also not an escape - a plaintext filename index discloses
precisely what encrypting filenames prevents, so it would need encrypting too, and searchable
encryption trades that for frequency and access-pattern leakage. The filter sits behind an
interface so a real index can replace it if a tenant ever grows into needing one.

**Retention follows membership, not the account.** An email is kept while the person holds any
active membership anywhere; the membership follows the role's own term, with mid-term updates
extending to the new expiry, which #3412's date-ranged model already expresses. When the last
membership ends the account lapses after three months - chosen so it survives the summer holiday,
since a parent whose seat ends in June and who is re-elected in August should be recognised rather
than start over. The correctness point: accounts are global across FAU-er, so the lapse test is "no
active membership anywhere", never "no activity in this FAU".

Erik raised member-elected retention from a real case - an FAU taking on helpers for a few weeks'
project, some expecting to return next year. Letting the person choose is better data protection
than a fixed rule imposed on them, but it turns retention into a per-person promise to store,
honour, expire and allow withdrawal. So the first migration carries a per-account retention field
with the default value and the MVP ships the fixed rule; election becomes a screen and a background
job later rather than a migration on live data. When it ships, an elected retention must expire and
be re-confirmed - otherwise "keep my data a year" drifts into indefinite - and withdrawal must take
effect promptly, because consent that cannot be withdrawn is not consent.

**The remaining timings.** Only the summer holiday restarts the confirmation clock; Christmas and
Easter are covered well enough by seven days plus monitoring every other day. Cancelling a queued
key destruction takes one operator, decided because there is currently only one person who could
act.

**Recovery contact.** Choosing neither is not permitted. Every FAU picks us or a school
representative, and **we hold the seat until a nominated representative is confirmed**, so the
transition has no gap. A nomination never confirmed leaves us in the seat, which is the safe
failure.

**The backup root key goes to Proton Pass**, a second trigger for #3481 alongside the SOPS age key.

**A standing risk named rather than closed.** One operator can cancel a destruction, one person
holds the Proton Pass credential, and the same person sits in the recovery seat for every FAU
without a confirmed school representative. That is the right call while there is one person, and it
is a concentration worth revisiting the day a second trusted operator exists rather than the day
one is needed.

## End-to-end encryption rejected, and FAU is not a reporting channel — 22 September 2026

Erik asked whether end-to-end encryption was possible and what it would cost, then decided against
it. Recorded with the reasoning rather than as a bare "no", because it is the obvious question and
will be asked again by someone who has not seen the analysis.

**Three reasons, in descending order of how decisive they are.**

Continuity is the product and E2E is incompatible with it. FAU exists so an archive outlives every
member leaving; under real end-to-end encryption, when the last key-holder goes the data is gone.
ADR-003's recovery contact is an escrow mechanism, and escrow is exactly what makes a system not
end-to-end. Continuity across total turnover or E2E, not both.

It would remove protection against the likelier attacker. Server-side validation of the document
JSON (#3490) and the whole ingest pipeline (#3447) require the server to read content; under E2E
they move into the browser or disappear. That trades defence against a compromised or malicious
member for defence against ourselves, and for a parents' council a bad upload reaching other
parents' browsers is the more probable event.

There is nothing to derive a key from. Passwordless login proves mailbox control and yields no
secret. A password reverses a core product decision, a separate passphrase is the second secret the
product was designed to avoid, and passkey PRF is ruled out for the MVP, unevenly supported, and
warned against for encryption by the WebAuthn specification's own co-editors because the data dies
with the passkey.

There is also an honesty problem at the end of it: we serve the JavaScript and hold the member key
directory, so it would be end-to-end against a passive server only. We would have built it and
still been unable to write "we cannot see your data" - a phrase this project already forbids.

**The partial version was dropped too, and that is a correction.** A "sensitive document" class,
client encrypted to current members, was proposed as worth keeping the door open for, on the
assumption that FAU would hold reports about named individuals. Erik's scope boundary below removes
that assumption. What remains - internal discussion, draft positions, a matter involving a named
child raised in a meeting - is already covered by encrypting bodies, titles and filenames. So the
document envelope needs no encrypted-to-members variant and #3490 stays simpler. The earlier
recommendation to reserve one is withdrawn.

**Scope boundary: FAU will never carry a whistleblowing or reporting channel.** Erik's reasoning:
an FAU should not be involved in reporting matters concerning the school, and a complaint against
an FAU member belongs with the school rather than with us. This is a statement about what the
organisation is for, not a feature deferred on cost. Nothing in the roadmap (#3433) should
reintroduce varsling, intake of reports about named individuals, or a confidential channel between
a parent and anyone outside the FAU. It is also what closes the end-to-end question rather than
merely postponing it, since the protections such a channel would demand are the ones that would
have justified the cost.

## Collaboration model decided: ProseMirror central authority, not CRDT — 22 September 2026

Erik and Sol produced an architecture brief proposing Tiptap OSS, ProseMirror, Yjs, y-prosemirror
and self-hosted Hocuspocus, with Yjs updates persisted in PostgreSQL. It was reviewed rather than
adopted, and the review changed four things. What survived is a better fit than either the brief or
the architecture it was replacing.

**The scope correction that drove everything else.** FAU-portalen is not a minutes archive. Erik
corrected a reading that had quietly narrowed the product: documents are the working surface for
everything an FAU does - an application for a new basketball court drafted by several parents over
weeks, a plan for the annual event that generates a task list this year and a different one next
year, with minutes as the simplest case rather than the typical one. Spreadsheets and presentations
are wanted later so an FAU has one place for its work. On that reading collaboration is a
requirement rather than a refinement, and #3420's exclusive-lease model was a compromise Erik
accepted only because he believed proper collaboration was out of reach.

**CRDT rejected in favour of a central authority.** Erik's instruction was to keep the existing
change-set model if it could support collaboration. It can: ProseMirror's own `prosemirror-collab`
uses a central authority that orders versioned step batches, with rebasing on the client. Three
reasons it beats Yjs here. Offline editing is explicitly out of scope, and offline is the advantage
a CRDT buys. History must be editable - Erik requires force-undo, removal from history and purging
whole documents, and a CRDT keeps deleted content as tombstones that garbage collection does not
retroactively purge from blobs already written. And a step batch is one blob with one author and
one timestamp, which drops onto #3412's audit model and ADR-003's encryption without friction,
where a CRDT update stream has neither an author nor a transaction boundary.

**Hocuspocus dropped, no Node in production.** The brief made it the default "unless there is a
strong architectural reason"; the standing decision of 11 September is that reason. The authority
is part of the Rust backend, which also puts collaboration authorization in the same process as
every other authorization decision.

**The server does not apply steps.** This was the largest risk in the first review - a Rust
reimplementation of ProseMirror's transform semantics that diverges from the JS produces silent
disagreement between server and clients. Erik's question removed it: clients apply steps and
materialise, the server orders, stores, broadcasts and authorizes. Server-side materialisation is
needed only at checkpoints and publication, and his publication proposal removed even that - the
publishing client renders to HTML and PDF, and the server sanitises what it receives with an
allowlist rather than rendering from a document model. The correction to his proposal, accepted:
the server sanitises rather than trusting the client's sanitisation, because under the threat
boundary below the publisher is exactly the person whose paste may have carried something they
never noticed. Published output is then served as a static file from a separate origin with no
JavaScript.

**Per-document keys.** Added to ADR-003 decision 5. Erik's requirement to purge whole documents
was going to be met with a cleaner that deletes rows from backups - unreliable by construction, and
the same mistake the original section 7 made. Instead each document has its own key, held in the
key service and wrapped under the FAU key-encryption key, so purging a document is destroying one
key and is provable everywhere including in existing backups. It also strengthens the granularity
property: a compromised backend must now ask once per document rather than once per FAU, so reading
an archive is loud in proportion to its size. In-document redaction remains a separate mechanism -
rewriting the step log from a checkpoint - which makes step-log compaction a privacy control rather
than only a performance one.

**Tasks are rows that point into a document.** Erik's annual-event case settled the direction: a
plan generates a task list for 2027 and a different one for 2028, so the document must not own
either. The deeper reason is encryption - document bodies cannot be queried, by the same
construction that rules out cross-FAU search, so anything the product must list across documents
has to exist as an application row. Tasks carry an anchor into the passage they came from, using
the same primitive as comment anchors: a position plus the version it was recorded at, mapped
forward through the step log.

**Document type and model version from the first migration.** The authority never interprets a
step, so it is model-agnostic. Two columns now mean spreadsheets and presentations reuse the whole
substrate - versioning, checkpoints, per-document keys, comments, anchors, permissions,
publication, encryption - rather than forcing a second one. Erik confirmed ProseMirror was never
intended to carry a spreadsheet; that will need its own editor and model, and is not being looked
at yet.

**Editor reversal recorded.** Tiptap OSS over ProseMirror replaces Lexical, superseding #3493 and
the editor clause of #3415, both of which are Done. The deciding reason is collaboration:
ProseMirror ships the model we are using with a decade of production behind it, where Lexical's
collaboration story is Yjs-shaped. Supporting reasons, which did not decide it: ProseMirror's node
extensibility is what decision blocks, action points and later document types need, and Erik
prefers to avoid Meta-originated dependencies where a comparable alternative exists.

**Import scope reduced.** MVP accepts HTML and pasted content; DOCX upload is deferred, which
defers rather than drops the mammoth recommendation on #3447. Paste is the primary path and carries
the same risk as upload, so normalisation runs on paste too. The normalisation layer sits inside
#3447's pipeline rather than beside it, and one contradiction with the brief is resolved
explicitly: images are re-encoded, never passed through.

**Threat model boundary**, set by Erik and now in ADR-003: we defend against mistakes and casual
misuse by people with legitimate access; we do not claim to withstand a determined attacker already
inside, and we never describe the product as secure against one. It justifies server-side
sanitisation of imported, pasted and published content, and scopes down adversarial hardening
against authenticated members.

**Consequences accepted and written down rather than discovered later.** Because the server does not
replay steps, a client-submitted checkpoint could disagree with the step log; the checkpoint is
authoritative for publication and recovery, the step log is history, and reconstructing history
from scratch needs a browser. PDF export in the publisher's browser means the same document
published by two people produces slightly different PDFs, so a published PDF is not a byte-stable
archival record. And published output sits outside the encryption boundary by design, so
crypto-shredding does not erase it - publication needs its own deletion path and its store must
permit real deletes.

Deferred: live presence and cursors, offline editing, track changes, DOCX and bulk import,
spreadsheets and presentations, and the agent edit API. Full architecture in
docs/structured-document-architecture.md, which replaces its 11 September version entirely.

## #3490 accepted: the document and collaboration architecture is settled — 22 September 2026

Erik accepted the architecture. docs/structured-document-architecture.md is the specification, and
it replaces its 11 September version in full. The decisions are listed in the card's result block
and in the section above; what matters for what happens next is which cards it unblocks and which
constraints it imposes on the first line of code.

**Binding on the first migration**, and expensive to change once data exists:

- Documents carry a `type` and a `model_version`, so spreadsheets and presentations reuse the
  substrate rather than forcing a second one.
- Tasks - and anything else the product must list across documents - are application rows that
  anchor into a document, never nodes buried inside an encrypted body. Encrypted bodies cannot be
  queried, by the same construction that rules out cross-FAU search.
- Each document has its own key, held in the key service and wrapped under the FAU
  key-encryption key. No key material of any kind lands in an application table.

**#3420 was closed the same day** and its surviving requirements moved into section 11 of the
architecture. The one worth remembering is that authorization is re-evaluated on every accepted
step batch rather than at connection establishment - the requirement that disappears silently when
a lease model becomes a WebSocket model, because a socket authorized at handshake otherwise
outlives the role that authorized it.

**Implementation is now unblocked except for one thing.** #3416, #3419, #3421, #3422, #3491 and
#3492 all have their architecture. The remaining gate on the first migration is #3439, still in
Review with four unanswered questions: two of them add `account.locale` and `tenant.default_locale`
to the schema, and a third - whether public pages carry a locale path prefix - binds routing. Those
belong in the first migration rather than a follow-up.

## Localisation settled, and ADR-003 accepted — 22 September 2026

**ADR-003 accepted** on #3484 without further comment. Identity, encryption at rest, deletion and
the recovery contact are now decided rather than proposed.

**Localisation answered** on #3439, closing the last gate on the first migration. The mechanism
proposed on 9 September was approved unchanged; what the answers settle is scope and two
consequences.

Nynorsk needs only the plumbing at MVP launch, so a translator is not on the critical path and no
`nn-NO.json` ships, not even a stub. The URL strategy is the recommended split: an outer locale
path prefix on public marketing pages, with the unprefixed path serving Bokmål so existing links
never change, and cookie plus `Accept-Language` resolution inside the authenticated app. A third
language is planned for - Erik named English or Sámi as possibilities. The translator round trip
uses a spreadsheet rather than XLIFF, on the grounds that it needs no specialist tooling from
whoever does the translation.

**Two things recorded that the answers imply rather than state.**

Per-school public pages (`example.no/kommune/skolenavn`) are neither marketing pages nor inside the
app, so under this decision they resolve by cookie and `Accept-Language`. One URL then serves both
languages and a Nynorsk school page is not separately indexable. Acceptable while Nynorsk does not
exist, and reversible - but only if #3422 and #3423 keep an outer prefix cheap to add and #3441
mints no municipality slug that collides with a locale code or ADR-001's reserved paths. Revisit
when a real Nynorsk catalogue exists.

Sámi is a separate language family rather than a written variety of Norwegian, and there are
several - Northern, Lule and Southern among them. So the pipeline must not assume every target
behaves like `nn-NO` does relative to `nb-NO`: plural rules, sorting and date formats differ more.
The BCP 47 model and ICU MessageFormat already carry this; the constraint is that nothing
downstream hard-codes two locales or assumes Latin-alphabet collation.

**The first migration is now unblocked.** It carries `account.locale` (nullable, follows the person
across FAU-er), `tenant.default_locale` (defaulting to `nb-NO`), and an optional
`document.language` for the `lang` attribute when rendering - alongside the three items #3490
binds: document `type` and `model_version`, tasks as anchored application rows, and no key material
in any application table.

## Clarification: "spreadsheets" meant the translator's file format — 22 September 2026

Recorded because the ambiguity is preserved in this log and would otherwise be re-derived. Erik's
"we start with spreadsheets" on #3439 answered that card's fourth question - whether the translator
receives XLIFF or a spreadsheet - and not the document-type roadmap. Confirmed by him the same day:
"since I was answering about translations to me spreadsheet could only mean that."

**Spreadsheet documents are not in the MVP.** That is unchanged from the document architecture on
#3490, which defers spreadsheets and presentations as document types needing their own editor and
model while reusing the collaboration substrate. The two uses of the word met in the same
conversation; they are unrelated.

## S3 credentials rotated, and the upstream defect it exposed — 22 September 2026

The leaked S3 credentials from #3489 are rotated. Erik created the new pair in the Hetzner Console,
wrote it into `.credentials.tfvars` and `.backend.hcl` himself so the values never entered a
transcript, and deleted the old pair. Stage 0 and stage 1 were re-initialised, planned and applied;
all stages now plan clean.

**What the out-of-order deletion actually cost.** Erik deleted the old pair before the node was
updated, rather than after. The consequence was contained and dated: the 13:00 etcd snapshot upload
failed - `readyToUse=false`, size 0 - while the local snapshot at the same minute succeeded. Off-node
backup was down for roughly fifteen minutes. Nothing else was affected.

**The deadlock this uncovered was not caused by the ordering**, and it is worth separating the two
because the instinct was to assume otherwise. Replacing `null_resource.k3s_master_install` runs a
destroy-time provisioner that calls `/root/backup-k3s-server.sh`, which begins `set -e` and runs
`aws s3 cp`. The AWS CLI on master returned `NoCredentials` - not `InvalidAccessKeyId` - so it has
never had credentials with any key pair. The module uploads three files and none of them configures
the CLI; the S3 credentials it does place on the node go to the k3s service's `EnvironmentFile`,
which a root shell never sees.

So the script fails before `systemctl stop k3s`, the destroy provisioner fails, and Terraform cannot
complete the replace - while the node's configuration is only rewritten by the *create* provisioner,
which cannot run until destroy succeeds. Any replace of that resource would have deadlocked, in any
order, with any keys, since the day the cluster was built. Written up in
docs/infra-tools-backup-script-no-credentials-issue.md.

**Resolution chosen by Erik: fix the credential availability rather than route around it.** The
alternatives were removing the resource from state to skip the destroy provisioner, or hand-patching
the node and accepting permanent `config_hash` drift. `/root/.aws/credentials` and `/root/.aws/config`
were written on master, mode 0600, from `persistent_outputs.json`. The apply then completed normally
and the destroy-time backup ran to completion - the first time it ever has. Those two files are
outside Terraform's management and are now listed in CLAUDE.md's secret locations; they must be
rewritten on any future rotation until the module is fixed.

**Verified afterwards:** all three nodes Ready with zero pod restarts, a forced etcd snapshot
uploaded with `readyToUse=true` on both the local and the S3 copy, and stages 0 and 1 plan clean.

**Stage 2 was deliberately excluded.** Its plan shows five resources to add - cert-manager, its
namespace, its ClusterIssuers, the empty Cloudflare token secret and the staging issuer - which is
#3485's pending work sitting in the uncommitted 2-cluster source, not rotation drift. "Rerun
everything" would have deployed cert-manager as a side effect. It remains unapplied.

## Delete protection on lb-fau: turned on by hand, fixed upstream later — 23 September 2026

`lb-fau` is the public entrypoint since the ingress-nginx pass: `fau-lab.bim.graphics` points at
its addresses, and the Hetzner CCM now manages its services. Deleting it would lose those addresses.
Deletion could come by hand, from a destroy of the wrong root, or from the CCM if the ingress
Service is removed. Hetzner's delete protection guards against that. It was the one open item on
#3485.

The upstream `bootstrap/network` module creates the load balancer without `delete_protection` and
has no variable for it. With hcloud provider 1.66.0 an unset value means `false`, so protection
enabled outside Terraform becomes permanent stage 1 drift.

**Decided by Erik:**
- He enables delete protection on `lb-fau` in the Hetzner Console now.
- The agent writes an upstream enhancement asking for a `load_balancer_delete_protection` variable:
  docs/infra-tools-lb-delete-protection-issue.md.

The alternatives were: wait for upstream with no protection meanwhile; or have the agent set it
through the API, which ends in the same drift and adds a live change made from the container.

**Consequence until upstream lands:** stage 1's plan shows one expected in-place change,
`delete_protection: true -> false` on `module.bootstrap_network.hcloud_load_balancer.nginx[0]`.
**Never apply that change.** Any stage 1 apply before the variable exists must be checked for it,
because applying it silently removes the protection. When the variable exists, set
`load_balancer_delete_protection = true` in `infrastructure/1-bootstrap`; stage 1 then plans
clean again.

## FAU creation and membership flow designed (#3413) — 23 September 2026

The flow that creates an FAU and brings people into it was designed with Erik on 23 September and
is written up in docs/fau-creation-and-membership-flow.md. The spec lists every decision. The ones
that change or clarify earlier entries:

- **ADR-003 decision 8, clarified.** In an FAU with no admin (no admin role valid and no handover
  grant valid), the recovery contact's single power, initiating the addition of a member, may grant
  an admin role. Outside that state it remains a plain member addition. All of decision 10's
  notifications apply.
- **Replacement proposals are in the MVP.** A member may propose a successor for their own role,
  and an admin approves it, which issues a normal invitation. This supersedes the 7 September
  "Confirmed simplifications" deferral for this one case. It shares one request model with the
  access request below. Other member-initiated invitations, and substitute invitations, stay
  deferred.
- **Passkeys are not in the MVP**; passcode only. This settles the inconsistency between ADR-003's
  "Closed" section and its decision 4a.
- **The leader invitation is exempt from #3414's TOTP gate.** It is issued in the activation
  transaction, because it completes the signup form rather than being a new admin action. Every
  later invitation is gated.

New decisions in the same session:
- **School selection.** The school is picked from the register (#3441), with the "Mangler skolen
  din?" fallback. The dependency on the register import is accepted.
- **One FAU per school**, enforced, counting pending FAUs. A duplicate attempt is copied to Erik.
  - If the existing FAU is active, the registrant is told to contact its admins, and the portal
    relays a passcode-verified access request without revealing who they are.
  - If it is pending, they are told it is being registered.
  - An unverified pending FAU expires after 7 days.
- **The first admin end date** is asked at signup. It defaults to the next 1 October at least three
  months away, within a range of 1–24 months.
- **A single admin is allowed**, with a banner while there is only one.
- **Invitations** are valid for 14 days, and opening a link never accepts it.
- **The named leader is taken on trust** in the MVP.
- **Migration 0003** brings forward minimal append-only `audit_events` and `outbox` tables, so
  activation does not wait for #3421.

## #3416, #3413 and #3485 accepted — 23 September 2026

- **#3416:** Erik accepted the backend skeleton and merged it to main via PR #1.
- **#3413:** Erik approved docs/fau-creation-and-membership-flow.md as written.
- **#3485:** Erik turned on delete protection for `lb-fau` in the Hetzner Console. The API
  confirms it, and a read-only stage 1 plan shows only the expected
  `delete_protection true -> false` drift. It must not be applied (#3497).

All three cards are Done. Erik then authorised overnight work, with his review in the morning:
- research the #3441 school register;
- plan migration 0003 and the membership domain that does not depend on the register, and build it
  test-first on a feature branch.

## Membership foundation built overnight: rulings and deferrals (#3418) — 24 September 2026

The register-independent membership foundation was built overnight on the branch
`membership-foundation-3418`, following docs/superpowers/plans/2026-09-24-membership-foundation.md.
Every task was reviewed, and a whole-branch review followed. The agent made these rulings on Erik's
behalf, and Erik reviews them.

**Rulings on behaviour:**
- **Frozen FAU.** Declining a request, lapsing requests, withdrawing an invitation and revoking roles
  or memberships are allowed. Each only closes something or reduces rights. This departs from the
  literal "writes stop" in ADR-003 decision 7a. Issuing, re-sending, approving, granting, proposing
  and recovery are refused.
- **Self-proposal.** A member may propose themselves as the successor for their own role. An admin
  still has to approve it.
- **Stepping down.** A member may revoke one of their own roles. Spec §7 already lets them leave
  entirely.
- **Request message.** It may contain line breaks (`\r\n` is normalised to `\n`). Every other control
  character is rejected. The limit is 500 characters, counted after normalisation.
- **Replaced token.** A token replaced by a re-send is answered as unknown, not as "replaced". The old
  hash is overwritten, so #3417's wording should cover both cases ("check your newest email").
- **Recovery notices.** Every recovery notice also goes to fau@ewb-solutions.as. EWB holds the seat
  only while no school representative is confirmed, so this matches ADR-003 decision 10 in every
  reachable state.
- **Outbox.** It carries a nullable `tenant_id`, for FAU deletion and Article 17 erasure.
- **Order of checks.** Authority is checked before any row state is revealed. Tenant state (frozen or
  closed) may be checked first, because members can see it anyway.
- **Base-image pins.** The Dockerfile pins multi-arch index digests (recorded 23 September).

**Deferred from spec 3413, not built yet:**
- §6.1 expiry warnings, and the §6.4 no-admin flag and notification sweep. Both need a scheduler.
- Recovery adding a plain member, outside the no-admin state.
- School-representative nomination: nominating, confirming and changing the seat. When it is built,
  the seat change must take the tenant lock, and recovery invitations must record the seat holder's
  identity, not only whether the holder is EWB or a school representative.
- The invitation preview read for the invitation page, and the one-admin banner query. Both come
  with #3417 and #3422.
- EWB recovery actions must record the acting operator in the audit entry before any recovery
  endpoint ships (#3417).
- The unauthenticated collision path, where each collision emails Erik, must be rate-limited at the
  HTTP layer (#3417).

**Decisions for Erik:**
- **The request message is stored as plaintext.** An access request's message (free text, up to 500
  characters) is on neither ADR-003 decision 6 list. Nothing writes it until #3417. The options are
  to encrypt it, drop the field, or add it to the plaintext list.
- **No upper bound on admin role periods.** Nothing limits the length of admin role periods set
  through invitations, grants or recovery. Only the first signup date is limited, to 1–24 months.

**Register design corrected (#3441).** Migration 0003 already creates the one-live-FAU-per-school
index, so the register migration adds only the `schools` foreign key.

## Access-request messages are sealed to the FAU — 24 September 2026

Erik chose sealing on 24 September. The access-request message (free text, up to 500 characters,
written by someone who is not a member) is encrypted to a per-FAU sealing key pair held in the key
service:
- The backend seals it on submission with the public key.
- It is decrypted only on the approval screen, inside an admin's session, by unwrapping the private
  key through the key service. The call is logged and rate-limited.
- It is never decrypted for email.
- Crypto-shredding covers it.

ADR-003 is amended to match: decision 6 gains a "Sealed to the FAU" category, and decision 5 notes
the key service's second narrow read operation. The rejected options were dropping the field, and
listing it as plaintext, which would leave strangers' free text in every backup and outside
crypto-shredding.

**Consequence for code.** Migration 0003's `access_requests.message text` column must become a
ciphertext column before 0003 is applied anywhere. Until the key service exists, the persistence
layer must accept no message at all.

## Invitation messages, readable by the invitee before accepting — 24 September 2026

Erik decided on 24 September that an admin may add a message to an invitation, and that the invitee
sees it on the invitation page after logging in and before accepting. The purpose is
anti-phishing: an invitation that says who is inviting you and why is harder to fake than a bare
link.

- **Encryption.** The message is encrypted under a per-invitation key, wrapped by the FAU's KEK. A
  database dump without the key store shows only ciphertext.
- **Reading.** The invitee is not yet a member, so the key service unwraps that one invitation's key
  only for a request that carries a valid, unused token from the logged-in, matching invitee. The
  call is logged and rate-limited. ADR-003 decisions 5 and 6 are amended.
- **Resend** keeps the message.
- **Considered and rejected:**
  - "Encrypting with the FAU's private key". Its protection would rest on a public key never
    leaking, which is fragile.
  - A key derived from the invitation token. It is stronger than the threat model needs, and it
    loses the message on resend.
- **Later, not now:** an option to include the message in the invitation email. That needs its own
  decision, because the plaintext would reach the mail provider.

**Code consequence:** migration 0003 gains a ciphertext column for the invitation message, beside the
access-request message change recorded above. Until the key service exists, neither message is
accepted.

## Key-service root key held by hand; member keys later — 24 September 2026

Erik decided on 24 September:
- **Live root key.** It is never on disk anywhere. Erik keeps it in Proton Pass and loads it by hand
  whenever the key service starts. Until then the key service is sealed: content stays unreadable,
  while login and authorization keep working. A sealed key service is a critical alert. The accepted
  cost is that a restart leaves content unreadable until Erik unseals it. This closes the case where
  the database and the key service's disk or backups are stolen together.
- **Member-held keys** are a post-MVP direction. The key service stores several wraps per FAU key
  from the start (KEK now; member devices and an offline escrow wrap later), so adding them needs no
  redesign. Adopting them needs passkeys and its own trust-model decision.
- **#3481** gains a third Proton Pass item: the live root key, beside the SOPS age key and the
  backup root key.

ADR-003 decision 5 and its closed items and standing risks are amended to match.

## #3418 accepted; no upper bound on admin role periods for now — 24 September 2026

Erik accepted #3418 on 24 September and merged the membership foundation (PR #2). On the open
question he answered: admin role periods set through invitations, handover or recovery need no upper
bound for now. Only the first signup date stays limited, to 1–24 months. He also confirmed the sealed
access-request message on the card.

## School register decided (#3441) — 24 September 2026

Erik answered D1–D11 in docs/school-register-design.md §12 on #3441 on 24 September. Most accept
the recommendation; four add to it.

**Accepted as recommended:**
- **D2** scope: the §2.3 filter (active grunnskoler, public and private, combined and special
  schools in; adult education, VGS-only and abroad out), a manual 2100 Svalbard entry, and a
  per-school operator override.
- **D3** municipality slugs use the Norwegian name: `0301-oslo`, `5540-kaafjord`.
- **D4** transliteration beyond æ/ø/å per §6. ADR-002 is amended.
- **D5** an operator can override a display name, and the sync respects the override.
- **D6** enumeration is read as §8 reads it. The register is open data and may be listed. What is
  protected is personal data, unverified submissions, pending state and membership. Outreach links
  are not secrets.
- **D7** unverified submitted schools stay hidden from the picker until verified.
- **D8** closures and re-registrations are automatic unless an FAU is attached, which makes them a
  review item.
- **D10** outreach links may carry one campaign-level parameter shared by every recipient of a
  mailing (`?kampanje=2026-10`), never a per-recipient one. Outreach itself still needs its own
  authorisation (#3426, #3427).

**Changed or extended by Erik:**
- **D1: NSR, Kartverket and SSB, plus Brreg's Enhetsregisteret for the FAU-er themselves.** Most
  FAU-er are registered in Brreg as their own entities. The register links each one to its school
  so that an FAU whose name differs from the school's is named correctly from the start. Matching
  is by address: an FAU and a school at the same address are a match, and several FAU-er at one
  address are flagged for manual inspection. The Udir `nxr-teknisk@udir.no` notice subscription is
  part of D1.
- **D9: a CronJob with its own `fau_register` role, but weekly, not daily.** A "Mangler skolen
  din?" submission triggers an immediate lookup in NSR. If the school is there, the submission is
  approved at once instead of waiting for manual review, because the school is now known.
- **D11: the `fau register review` CLI plus email for the MVP.** The admin web screen (b) is high
  priority immediately after, on #3499.
- **Merging FAU-er when schools merge (new requirement).** When two schools combine, a new FAU for
  the merged school gets the old FAU-er's documents shared into it. Members keep read-only access
  to the old FAU-er and continue in the new one. This supersedes §4.5's "The product does not merge
  FAU-er". It depends on the document layer (#3419) and per-document keys (ADR-003 decision 5), so
  it is its own card, #3498. The register only has to record the merger (many closed schools, one
  successor), and its schema already allows that.

**Agent rulings on the new parts, for Erik's review.** Measured on 24 September 2026 against a full
Brreg bulk download and a full NSR grunnskole download, both discarded afterwards:
- **Brreg addresses are personal data in practice.** 913 of the 2,477 FAU-like entities carry a
  `c/o` or `v/` line, which usually names a person at a home address. So addresses are used only
  in memory, during matching. They are never stored, logged, placed in a review item or shown.
  Stored per entity: organisation number, registered name, organisation form, municipality number,
  status and the match outcome.
- **Which entities count as an FAU.** Organisation form FLI with a name containing FAU,
  "foreldrenes/foreldrerådets arbeidsutvalg", "arbeidsutval(g)" or "foreldreråd(et)", matched on
  word boundaries: 2,477 entities today. The Brreg search API cannot serve this. Its `navn` filter
  is not a substring match, and FLI with NACE 94.992 is 73,355 rows, past its 10,000-row paging cap.
  So the weekly sync streams the bulk file `enheter/lastned` (210 MB gzip, 1.18 million units,
  about 40 s) and keeps only those entities.
- **Address matching.** The FAU's street line and postcode are compared with the school's visiting
  and postal address, after normalisation. `c/o`, `v/` and post-box lines are ignored. Unique in
  both directions (one FAU, one school) links automatically: 1,256 today. Where the name also
  identifies a single school, the two agree 876 times and disagree 4 times. Everything else goes to
  review:
  - several FAU-er at one school: 28 schools;
  - one FAU address matching several schools: 29;
  - address and name disagreeing.

  **Name-only matches (360) are stored as unconfirmed candidates**, not linked and not queued. They
  wait for the admin screen (D11b, #3499), so the seed does not flood the queue.
- **What a link does.** At signup the FAU name defaults to the linked entity's registered name,
  case-normalised because Brreg stores names in capitals, and otherwise to the school's display
  name. It is a suggestion the registrant edits, like every other prefill. The link grants nothing.
- **Exception, decided by Erik the same day:** a `c/o` or `v/` line that names the school counts,
  and one that does not stays excluded. "c/o Hosle skole, Bispeveien 73" is evidence for Hosle
  skole. "c/o <a person>" or a line naming another school is not. Such a line is checked only
  against a specific candidate school sharing the postcode, and its key counts only for that
  school.
- **The seed sends one summary email**, not one per review item. Later runs email each new item.
- **The submission lookup keeps D9's privilege line.** The submission queues a lookup row, which
  the runtime role may insert. `fau register lookups`, running as `fau_register`, reads NSR's
  list for that municipality and processes the queue. It runs every few minutes as a CronJob
  (#3424), so "immediately" means within minutes. Approval is automatic only when exactly one
  active, in-scope NSR school in the submitted municipality, not already in the register, has the
  same folded name. The submitted school then absorbs the NSR data, is verified, gets its slug,
  and Erik is told rather than asked. A similar but not identical name, or a match with a school
  already listed, becomes a review item.
- **Weekly timing.** Mondays at 04:30 Europe/Oslo, after NSR's nightly Brreg import. The circuit
  breaker keeps its thresholds of 2% closed and 5% renamed per run.

## Register foundation built: rulings for Erik's review (#3441) — 24 September 2026

Part 1 of the register was built on the branch `school-register-3441`, following
docs/superpowers/plans/2026-09-24-register-foundation.md. It covers migration 0004, the
`fau_register` role and the pure register rules in `fau-domain`. Every task was reviewed, and a
whole-branch review followed. Nothing has been seeded or applied to any environment. The agent
made these rulings on Erik's behalf:

**Schema (0004 is unapplied, so all of these are still cheap to change):**
- **Row-level security confines the runtime role on three tables.** On `schools`, `fau_app` may
  insert only a fresh, pending, submitted row. On `school_submissions`, only a pending submission
  that no reviewer has touched. On `register_lookups`, only an unprocessed lookup with no
  attempts. D9 says the runtime role must not be able to assert register outcomes.
- **`fau_register` may delete only `held` schools.** This guards against mistakes, not against
  the role itself, which can relabel a row.
- **Review items point at schools with `on delete set null`.** A held row named in a
  `possible_submission_match` can then be deleted when the match is resolved (§4.5). Its orgnr and
  name must be kept in the item's `details`. The sync must write neither orgnr history nor FAU
  links for a held row, because those references would block the delete again.
- **Checks added beyond the plan:**
  - `scope_reason` is limited to the seven domain codes;
  - slug format and length checks on every slug column;
  - review-item `details` capped at 2 KB;
  - `fau_register` may delete municipality names that Kartverket drops.

**Brreg rules:**
- **FAU words are exactly §2.5's list.** "Samarbeidsutvalg" and "foreldreutvalg" were removed:
  a samarbeidsutvalg is a different statutory body, and one registered at the school's address
  would block the real FAU's link.
- **A line is personal wherever its marker sits.** That covers a `c/o` anywhere, a `v/` before
  the house number, and a `v/` after the house number that is followed by more words. The cost is
  losing some real addresses, such as "V. Slottsgate 2" and "Storgata 12 V 2". Privacy wins over
  recall. A lone house letter ("Storgata 12 V") still counts.
- **What counts as naming the school, under Erik's `c/o` exception.** The line must contain the
  school's full name, or its distinctive part followed by a school word ("Hosle skule" for Hosle
  skole). A line that names the school but has no street and number yields no key.
- **Addresses cannot reach a log.** `AddressKey` prints its street as `<redacted>` in debug
  output. Keys may only ever be compared for exact equality, and never logged or stored. The
  matcher plan inherits that rule.

**Search and scope:**
- **An empty search matches nothing.** A query with no letters or digits folds to "", and the
  search plan must then return no rows rather than run the match.
- **`primary_nace` picks the priority-1 NACE code.** It is tested on a combined school, since 50
  combined schools depend on it.

## FAU contact e-mail: fetched live by outreach, never stored in the product (#3441, #3431) — 24 September 2026

Erik wants to contact FAU-er registered in Brreg and invite them to FAU-portalen. He decided on
24 September:
- **The product stores only the FAU's organisation number.** `registered_faus.orgnr` is the key,
  so the Brreg record can always be retrieved again. `fau register export` lists the linked FAU's
  orgnr for each school.
- **The contact e-mail is fetched live from Brreg by the outreach tooling (#3431), at send
  time,** and kept there under #3426/#3427's retention. It never goes into the product database,
  and the register's source parser never reads Brreg's `epostadresse`, `mobil` or `telefon`
  fields.
- **Why:** Brreg's FAU e-mail is often a parent's private address. Two of nine sampled records
  had a private Gmail or Hotmail address. Keeping it out upholds "the register holds nothing about
  individuals", and keeps parents' addresses out of every product backup.
- **Still required before any mail is sent:** Erik's authorisation of outreach, and the legal
  check on #3427. Markedsføringsloven §15 generally forbids unsolicited marketing e-mail to natural
  persons without consent. Whether a private address registered for an FAU (a legal person)
  counts as a natural person's is exactly the question that check must answer.
