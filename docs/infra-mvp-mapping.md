# FAU MVP mapping to infra-tools

Review date: 7 September 2026. Evidence is the local checkout only; no Terraform plan, cluster deployment, image build, or recovery exercise was performed. Current supplier prices, ownership and product availability have not been verified by this code review. User answers in planning-decisions.md, with the latest clarifications taking precedence, are requirements. Architecture suggestions below are proposals, not user decisions.

## Main finding

infra-tools supplies a reusable deployment and operations foundation. Its demo is a Django visitor logger, not an FAU product starter. Rust/TypeScript can fit the same container, PostgreSQL, ConfigMap/Secret, Service, ingress and Flux interfaces. Application authorization, traceable document history, member-visible audit records and magic-link authentication must be built.

Erik clarified after this inspection that the project will follow the existing infra-tools setup with three servers. It defines a CPX12 VPN router, CPX22 master, CPX22 worker and LB11 load balancer. The database defaults to two instances that must occupy distinct worker-labelled nodes. Verify and cost this topology; the earlier one/two-node preference is superseded. No reduced-topology redesign is requested.

## Evidence and mapping

Paths below are relative to /workspace.

| User requirement | Existing code evidence | Required project work |
|---|---|---|
| Rust backend, TypeScript frontend | `infra-tools/demo_app/Dockerfile`, `demo_app/ep.sh` build Python and start Django/Gunicorn; `gitops/deployment.yaml` supplies `serve`, port 8000, configuration and secrets | Build a Rust image with matching runtime contract; choose lightweight TS UI through a bounded comparison. No requirement to retain Django. |
| Local container workflow | `infra-tools/demo_app/docker-compose.yml` runs app plus PostgreSQL 17.5 | Supply project Compose development setup, migrations and `.env.example`; use development-only credentials and no copied external service endpoints. |
| Kubernetes release | `infra-tools/.github/workflows/build_image.yml` builds GHCR image, tags branch/SHA and commits the manifest; `infrastructure/3-app/configuration.tf` points Flux to an author repository and `version-3` branch | Replace project names/repository/branch and review provider ownership scope. Add meaningful tests, migration sequencing, health probes, resource requests, release/rollback checks. The demo deployment has no readiness/liveness probes or resource requests. |
| Shared accounts, FAU switching, verified access, admin-only invites | Demo uses Django stock auth settings; visitor view stores IP/user agent (`demo_app/project/visitors/views.py`) | New tenant/account/membership model; enforce tenant isolation server-side for every read/write; verified email required before access; login choice and switching dropdown; leader and registrant receive admin only after their own verification. No magic-link implementation found. |
| Immediate signup and notification | No corresponding demo workflow | Collect FAU name, school name, municipality, municipality school-page URL, registrant and leader emails; activate after registrant verification; notify `fau@ewb-solutions.as`. Research European-owned mail delivery. |
| Time-limited roles and restricted handover | No corresponding demo model | Any current role retains normal access; expired admins get only six-month handover actions (invite, assign roles, grant admin). Enforce dates and permission limits in backend. Initial school setup required; subsequent transitions deferred. |
| Markdown, txt/md upload, autosave and stored change sets | Demo only stores visitor records | Build tenant-scoped documents and immutable changes from first save. Transactionally couple current document, change record and audit event; choose diff/snapshot representation explicitly. No history UI required. Validate text imports; safe Markdown rendering. |
| Exclusive editing, 15 minutes without changes, admin force release | No corresponding demo workflow | Server-managed lease and fencing token/revision validation. Closing releases lock; no document changes for 15 minutes releases; forced release uses latest persisted content and discards unsaved content. Reject later stale saves from revoked editor. |
| Every current member sees FAU audit | JSON stdout/OTLP in `demo_app/project/main/settings.py`, `infrastructure/3-app/config_maps.tf`; full telemetry modules in stage 2 | Operational logs/traces do not supply tenant-aware business audit history. Implement actor, organization, action, target, timestamp and change reference; authorized audit view. Avoid magic tokens/document bodies in operational logs. Traceability must survive normal telemetry retention. |
| S3 originals optional | `infrastructure/3-app/storage.tf` creates static bucket; Django settings mark static objects `public-read`, unsigned; `3-app/main.tf` exposes static gateway | Static asset pipeline is unsuitable as private document authorization. If originals are stored, provide private tenant-scoped storage/access checks and retention/deletion design. Do not put FAU documents in the public static path. |
| Backup/recovery | `shared-modules/psql-cluster/main.tf` sets Barman S3 WAL compression, daily 02:00 backup, recovery source; `variables.tf` defaults to 30d retention and 10Gi per instance. `shared-modules/k3s-master/k3s-config.tpl.yaml` configures etcd S3 snapshots | Existing mechanisms are substantial but do not prove this project's recovery. Decide RPO/RTO, validate fresh bootstrap and backup names, restore into isolation, verify tenant documents/change sets/audit, document disaster procedure. |
| GDPR and European ownership | Infrastructure provides transport/storage/secret primitives, plus external provider integrations | No complete application deletion, data inventory, processor terms, lawful-purpose/retention policy or rights handling found in demo. Record data flows and ownership evidence for services before selection. This review does not establish GDPR compliance. |
| Full Bokmål landing page and signup | Visitor-list HTML only | Create product copy and functioning signup flow; product name/domain remain open. Payment later; no KFAU signup work in MVP. |

## Proposed repository and runtime shape

A small modular application, using the infrastructure modules as deployment dependencies:

```text
backend/              Rust Cargo workspace; API and domain modules
backend/migrations/   versioned PostgreSQL migrations
frontend/             TypeScript UI and Bokmål landing page
Dockerfile            frontend build + Rust build + minimal runtime
compose.yaml          local application and PostgreSQL
infrastructure/       project-specific stage configuration/module references
gitops/               deployment/service/configuration templates
docs/                 decisions, data map, operations and recovery evidence
```

A single production application image serving the built frontend is a proposal to minimize pods and configuration; separating frontend delivery remains possible after evidence. Keep one backend with internal modules for identity, tenancy, roles, documents, leases and audit. The existing infra-tools setup includes RabbitMQ; preserve that setup. A separately operated identity server remains an unresolved option. A durable database-backed email outbox is a candidate, not yet selected.

Preserve the useful demo contract: port 8000, `serve` command, database environment variables from ConfigMaps/Secrets, structured stdout and OTLP. Replace Python-specific OTEL variables/instrumentation with Rust instrumentation. Use a deliberate one-at-a-time migration step rather than blindly migrating concurrently whenever replicas start. Add a readiness check that reflects essential dependencies and a liveness check that does not restart healthy processes merely because PostgreSQL is temporarily unavailable.

## Infrastructure decisions needed before deployment

1. **Existing three-server topology and total bill.** Follow the checked-in VPN/master/worker and load balancer setup. Verify current official prices, availability, volumes, IPs, object storage and email costs, then measure capacity. Do not remove or consolidate nodes based on the superseded one/two-node preference.
2. **Operational overhead.** Stage 2 enables full Grafana/Loki/Tempo/VictoriaMetrics/Alloy, operators and RabbitMQ operator; stage 3 creates RabbitMQ. Inventory resource use, verify the existing configuration and document its measured requirements. Operational logs remain separate from business audit even if telemetry is reduced.
3. **Provider ownership and boundaries.** Source integrates Cloudflare DNS/ACME, GitHub/GHCR, Slack notifications, SMTP2GO sample alert transport and ntfy. These are example dependencies, not approved FAU suppliers. Determine whether the ownership requirement applies equally to build/coordination tooling as to runtime/data processors, research actual ownership, and propose substitutions where needed. Domain and certificate configuration cannot be finalized while domain is unknown.
4. **Fresh install safety.** Stage 3 still names demo app, author domains/repo, sample restore source `psql_0_backup`, and `{}` bootstrap while module supports explicit fresh initdb. Review rather than copy these values. `3-app/storage.tf` and `2-cluster/storage.tf` allow `force_destroy` with commented destroy prevention; define production safeguards and data ownership. No destructive operation was run.
5. **Release reliability.** GitHub workflow reacts to path changes across pushes and writes deployment manifests automatically. Define target branch/environment, review gates and rollback semantics. Add application tests for cross-tenant access, verification, role expiry/grace, concurrent autosave/lock revocation and atomic audit history.

## Work packages for Favro

- Architecture/container contract and lightweight frontend decision.
- Hetzner existing three-server capacity/cost and availability research, including unavoidable ancillary services.
- European-owned mail/authentication and runtime supplier assessment; magic links remain preferred.
- Identity, organization signup and notifications; tenant isolation and switching.
- Invitations, time-limited roles, handover and initial school configuration.
- Markdown/txt/md documents, transactional stored change sets, autosave and fenced edit leases.
- Business audit schema and member-visible audit UI, tied to changes.
- Private object storage decision (nice-to-have originals), separate from static assets.
- Recovery exercise and GDPR/data lifecycle evidence, including retained history/backups.
- Bokmål landing page and launch identity/domain decision.
- CI/release hardening and end-to-end acceptance evidence.

Acceptance for the infrastructure package must include an actual fresh install and isolated restore rehearsal before claiming deployment/recovery readiness; this inspection establishes code coverage only.
