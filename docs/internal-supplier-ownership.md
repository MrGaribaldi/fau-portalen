# Internal suppliers: ownership and data boundaries

Date: 9 September 2026, notification endpoint added 10 September 2026. Owner: Infrastruktur,
drift og lagringsøkonomi. Favro: #3442. Status: recorded; Erik confirmed the three open questions
and accepted Healthchecks.io with Signal as the alerting endpoint on 10 September. This is a
record and a set of boundary rules, not a set of approvals.

Scope is FAU's **internal** suppliers: the ones that never sit in the path of FAU users or their
data. Customer-facing suppliers are #3409 and must be European. Per the supplier ownership policy
of 9 September, internal services may be non-European; Canadian ownership is explicitly
acceptable, and American AI tooling is already in use and accepted. Nothing here re-litigates
that.

## The rule a person can follow

Four things must never reach any supplier on this page:

1. **FAU member data** — names, contacts, roles, documents, minutes or anything identifying a
   parent, pupil or school contact.
2. **Credentials** — API tokens, S3 keys, passwords, `.credentials.tfvars`, `favro.env`.
3. **Terraform state and generated keys** — `terraform.tfstate`, `persistent_outputs.json`, the
   admin SSH key, the WireGuard keys, the SOPS age private key.
4. **Anything derived from the three above** that would reconstruct them, such as a log excerpt
   containing a token or a database dump containing members.

Everything else — architecture, plans, decisions, source code, infrastructure configuration — is
free to move through these services. That is what they are for.

## Decide need before mapping

Erik's instruction on #3406, 9 September: "Later we should go through and see which extra
services, like slack and ntfy are actually needed." A service that is dropped needs no ownership
record, no boundary rule and no maintenance, so need comes first. The following appear in
upstream infra-tools as example integrations and are **not adopted by FAU**:

| Candidate | Where it comes from | Status |
| --- | --- | --- |
| Slack | Upstream alert notifications | Dropped, confirmed by Erik 10 September 2026. No FAU account, no webhook configured. |
| ntfy | Upstream push notifications | Dropped, confirmed by Erik 10 September 2026. |
| SMTP2GO | Upstream sample alert transport | Dropped, confirmed by Erik 10 September 2026. Do not confuse with #3410, which is the product's transactional email and is customer-facing. |

No ownership record is written for these three until a decision to adopt one exists. All three
overlap with alerting that the in-cluster telemetry stack already provides and with the Favro
watcher proposed on #3436, so the likely answer is that none is needed. Source:
`docs/infra-mvp-mapping.md`, item 3, which already records them as "example dependencies, not
approved FAU suppliers".

## Suppliers actually in use

### Favro — coordination surface

Favro AB, Uppsala, Sweden; European. The platform is the project's coordination surface: boards,
cards, comments, attachments.

Hosting and subprocessors, from Favro's own DPA and subprocessor list, both last updated
6 August 2026:

- Infrastructure hosting: Cleura (City Network) in Sweden and Frankfurt, and AWS in Frankfurt —
  EU/EEA. Backups span both providers.
- Intercom, US, for customer support, under adequacy decision (DPF) and SCC.
- Mailgun, US, for the Email2Card feature, under adequacy decision (DPF) and SCC.

Favro processes as data processor under a DPA that forms part of its terms.

**Boundary.** Cards describe the project, not its users. Card text, comments and attachments must
stay free of member data — this is the one supplier on this page where accumulation is genuinely
plausible, because a pilot conversation or a bug report is exactly the kind of thing someone
pastes into a card. Two consequences to keep in mind: opening a support conversation routes card
content through Intercom in the US, and the Email2Card feature would route mail through Mailgun,
so it should stay unused.

### GitHub and GHCR — source hosting and container registry

GitHub, Inc., a Microsoft subsidiary; United States. Used for source hosting, the container
registry, and CI. The agent image is pulled from `ghcr.io/akantodevs/agent-box:v1.4.2`, and
`sops` is fetched from GitHub Releases at a pinned, checksummed version.

**Boundary.** Public repositories must contain no credentials and no state; `.gitignore` in
`infrastructure/` already refuses `*.tfstate`, `*.tfvars`, `*.tfplan` and `.config/`, and
`/workspace/CLAUDE.md` is deliberately never committed. GitHub Actions secrets are a credential
store and belong to the GitOps work on #3424, not here.

### Anthropic Claude and OpenAI Codex — AI development assistants

Anthropic PBC and OpenAI, both United States. Accepted by Erik for development and planning.
Claude runs the agent sessions in this container; Codex is optional and may not be installed.

**Boundary.** Whatever an agent reads can leave the container in a request to the provider, and
the session transcript persists in the state volume. So: no member data, and no credentials or
state in commands or output. `/workspace/CLAUDE.md` already carries the operational form of this
rule — read secrets indirectly through `$VAR`, never echo them.

**This is the dependency most likely to drift.** Today the workspace holds planning documents and
configuration only, so nothing crosses the line. That changes the first time someone debugs
against real pilot data, or reads a production database into an agent session. At that moment the
AI assistants stop being internal tooling and the question returns to #3409 as a new decision.

### Upstream infra-tools and agent-box — third-party source

`infra-tools` is a private repository and `agent-box` a published image, both from the third-party
`akantodevs` organisation on GitHub. FAU consumes the Terraform modules read-only from
`/opt/infra-tools` at a pinned commit, and extends the agent image through FAU's own
`Dockerfile.agent`.

Legal owner is not established here and the account is not FAU's. This is a supply-chain
dependency rather than a service: nothing is sent to it, and code is pinned by commit and digest.
It is recorded so the dependency is visible, and because the FAU-owned Terraform root exists
precisely so upstream changes cannot alter FAU's infrastructure unnoticed.

### Build-time package sources

crates.io (Rust Foundation), the npm registry (GitHub/Microsoft), PyPI (Python Software
Foundation), rustup, and Debian package mirrors. US-based foundations and distributed mirrors;
all internal, none receives FAU data.

**Boundary.** These are inbound only. The control that matters is not ownership but integrity:
pinned versions, lockfiles committed, and checksums where a download is not from a package
manager, as `Dockerfile.agent` already does for `sops`. Dependency provenance for the application
itself belongs to #3424.

### Hetzner — customer-facing, listed here for one internal use

Hetzner Online GmbH, Germany; European, and assessed on #3409 as a customer-facing supplier.
The internal use worth naming is the private `fau-tfstate` bucket, which holds Terraform state,
and therefore holds the admin SSH key, the WireGuard keys and the age key in clear text. That is
FAU's most sensitive object store and it is deliberately not shared with anything on this page.

### Monitoring and alerting

Grafana, Loki, Tempo, VictoriaMetrics and Alertmanager are self-hosted in-cluster, so they are
not suppliers — no telemetry leaves the cluster today, and nothing is running yet in any case.

**The supplier is the notification endpoint**, and it is now decided rather than open.

### Healthchecks.io — the alerting endpoint outside the cluster

Accepted by Erik on 10 September 2026 (#3442). Healthchecks.io is operated from Latvia, inside the
EU; the legal entity name belongs in this record and should be taken from their DPA when the
account is actually created, rather than asserted from memory here. It receives two kinds of
traffic: Alertmanager's webhook receiver POSTing to a check's `/fail` endpoint for critical
alerts, and a scheduled ping from the always-firing Watchdog alert whose *absence* is the alarm.
That dead man's switch is the reason an external endpoint exists at all — a dead cluster cannot
alert on its own death.

Warning-severity alerts do not go here. They route to email through the managed European provider
being chosen on #3410, which is a customer-facing supplier assessed on #3409.

### Signal — the transport for critical alerts, and non-European

Signal Technology Foundation and Signal Messenger LLC are **American**. Healthchecks.io delivers
critical notifications over Signal by running a real `signal-cli` client, because Signal has no
incoming webhooks. So the supplier is Latvian and the transport is not, and that is recorded as a
deliberate choice: it is permitted by the internal-supplier rule because it carries scrubbed
operational content and never FAU member data, but it is not European and should not be described
as if it were.

**The boundary rule here is stricter than elsewhere on this page, and it covers metadata.** An
alert body can carry user identifiers out of the cluster through a log line, a trace or an error
message, so alert content stays restricted to service-level facts. Operational telemetry is
separate from the business audit log by design — see `docs/infra-mvp-mapping.md` — and an external
alerting service must never become a route by which audit content leaves. **The same applies to
check names**, because with this design the check name is what actually reaches the phone: a name
like `fau-prod-db-down` is fine, a name carrying a school, a person or a document identifier is
not.

Practical notes accepted with the decision, from the Healthchecks maintainer's own write-up: the
first message must be sent from the account's own side or Signal rate-limits it, some accounts
have needed a manual CAPTCHA step before notifications start, and delivery takes seconds rather
than being instant. First setup is therefore a hands-on step, not pure Terraform.

## Escalation

One trigger, and it is the same for every entry: **a supplier starting to handle FAU member
data.** That moves it out of this record and into #3409 as a new decision, per the ownership
policy. The two realistic candidates are Favro, through pasted content, and the AI assistants,
through debugging against real data.

## Answered by Erik, 10 September 2026

1. Slack, ntfy and SMTP2GO are dropped rather than mapped, as recommended. None is adopted, and
   in-cluster telemetry plus the #3436 watcher covers the need.
2. An alerting endpoint outside the cluster **is** wanted, and it is Healthchecks.io with Signal
   as the transport. Both now have records above.
3. No internal service in use is missing from this record. The standing caveat remains that
   anything reached from Erik's own machine rather than from this container is invisible here, so
   this is true as of the date above rather than permanently.

## Still open, as a test rather than a decision

How much of an alert body survives into the Signal message. A Healthchecks notification is built
around the check name, and the POSTed body may or may not be included. If it is not, the design
still holds but the check names have to carry the meaning — one check per alert class rather than
one check for everything. Ten minutes of testing before Alertmanager is wired to it.
