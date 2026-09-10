# FAU-plattform agent operating model

Date: 7 September 2026. Status: proposed execution specification; this document does not start background workers or authorize implementation, spending, deployment, publication, or outreach.

Sources: `prosjektgrunnlag.md` section 17, Erik's interview answers summarized in `docs/planning-decisions.md`, and `.agents/skills/favro/SKILL.md`. Later explicit answers supersede earlier answers. Unknowns remain questions, not inferred requirements.

## Coordination and execution

Favro is authoritative for ownership, status, dependencies, questions, reviews, and results. Code, research datasets, specifications, and decision records stay in their appropriate repositories and are linked from cards. The collection is private to Erik W. Bjønnes and the required agent account.

Use one stable board per role and one primary role per card. Runtime agents are temporary workers for those roles, not additional boards. Agent comments begin exactly with the role's emoji and label followed by a colon. Preserve the ten original roles and their emojis below. Route all human questions to Erik through the pinned `👤 Needs you` checklist, with one interview topic per card.

Workflow: Inbox → Backlog → Ready → In Progress → Waiting → Blocked → Review → Done. Waiting means expected input or dependency; Blocked means an unexpected impediment. Done requires acceptance evidence, a recorded result, and review. Do not mark planned work as implemented. Do not overwrite human comments.

Before a worker starts, give it the owning card, current source documents, scope, allowed actions, dependencies, files it owns, acceptance conditions, reviewer, and stopping conditions. Claim the card before editing; keep one writer per artifact. Independent work can run in parallel; dependent decisions must be resolved first. Return findings and reviewable artifacts even when another part waits for Erik. Record actual test/research evidence and distinguish observations, proposals, decisions, and unknowns.

## Original roles

| Role and board | Inputs and responsibility | Concrete outputs and review |
|---|---|---|
| 🧭 Produkt og prosjektledelse | Interview answers, delivery target, all workstream findings. Maintain MVP boundary, dependency order, decisions, and questions. | Traceable scope, prioritized cards, release checklist and human handoffs. Erik resolves product choices; specialist owners review their requirements. |
| 👥 Brukerbehov og FAU-organisering | FAU workflows and confirmed membership rules. Specify initial organization setup, shared administration, verification, valid-role access and handover grace permissions. | User journeys, permission examples and dated-role scenarios. Product and security review. No pilot interviews or recruitment messages without authorization. |
| 🧱 Systemarkitektur og datamodell | Confirmed requirements and inspected infra-tools contracts. Specify Rust API, PostgreSQL tenancy, organization history, document change sets, audit events and exclusive editing. | Architecture decisions, schema/API contracts, application-to-infrastructure mapping, migration strategy. Infrastructure, security and quality review. Do not select an unresolved vendor/framework silently. |
| 🔐 Sikkerhet og GDPR | Data flows, suppliers, application/infrastructure evidence and applicable primary sources. Assess tenant isolation, sessions, invitations, retention, deletion, audit integrity and recovery. | Threat model, data inventory, security acceptance checks and concrete privacy gaps. Legal reviews obligations; quality verifies controls. Infrastructure logs are not assumed to satisfy application audit requirements. |
| 💾 Infrastruktur, drift og lagringsøkonomi | infra-tools source/demo, proposed app contracts, confirmed existing three-server infra-tools setup. Verify suitability, actual available resources, full cost and operational coverage. | Container/deployment contract, dated sizing/cost research, backup/restore and incident runbooks, explicit coverage gaps. Architecture/security/quality review. No numeric spending cap is inferred from the node preference or product price. |
| 🎨 UX og universell utforming | Confirmed flows, Norwegian Bokmål only, simple frontend requirement. Design landing page, signup, organization switching, member setup, editing and visible save/lock states. | Accessible flows, copy, UI specification and lightweight TypeScript framework evaluation. Product and quality review. Product identity/domain remain open. |
| 💰 Forretningsmodell, marked og prising | NOK 200 + VAT/month billed annually, no signed pilots, market sources and cost findings. Own detailed market mapping, competition, segmentation and evidence-based prioritization. | Market report with dated primary-source evidence, explicit estimates and methodology; segment priorities handed to sales. No invented demand, adoption rates or verified-contact claims. |
| ⚖️ Juridiske rammer og avtaler | Product/data flows, supplier agreements and current authoritative legal sources. Analyze controller/processor allocation, supplier terms, privacy information and contracts. | Source-linked legal questions and reviewable terms/DPA drafts; findings for Erik and security. Do not describe drafts as approved legal advice or claim GDPR compliance without evidence. |
| 🧪 Kvalitet, test og risiko | Acceptance conditions, code, deployment contracts and security requirements. Independently verify the MVP and review release evidence. | Focused test matrix, regression/security results, prioritized defects and release recommendation. Include cross-tenant access, role expiry, grace restrictions, stale lock writes, autosave/version/audit consistency and restore evidence. |
| 📣 Salg, pilotering og kundekontakt | Market segmentation and public institutional sources. Own national FAU prospect discovery, deduplication, pilot preparation and later authorized customer contact. | Provenance-backed prospect register, coverage report, contact verification status, pilot scripts and draft communications. Market/security/legal review relevant findings; Erik authorizes external contact. |

## Implementation roles configured for future code execution

These two stable workstreams separate implementation ownership from architecture and independent review. They do not replace the original ten roles. Their boards and emojis are configured now so implementation cards have explicit owners; this does not start implementation workers.

| Proposed role | Inputs | Outputs and dependencies |
|---|---|---|
| 🦀 Backend og integrasjon | Reviewed data/API/deployment contracts and scoped code cards. | Rust service, migrations, magic-link integration, tenant authorization, dated roles, document/change-set/audit transactions, lock enforcement and tests. Depends on security/auth decisions and email-provider contract; architecture and quality review. |
| 🖥️ Frontend og nettsted | Reviewed UX, API contract, chosen lightweight TypeScript framework and code cards. | Bokmål landing page, signup/login, organization selection/switching, initial setup, admin invitations, editor/autosave/locks and audit view. Depends on confirmed identity where needed and API contracts; UX and quality review. |

If these roles are not enabled, explicitly assign bounded implementation to an existing role and name a different reviewer; do not leave code ownership implicit.

## Market and national FAU discovery work package

Market analysis and prospect discovery can proceed before MVP completion. Sending recruitment messages waits until MVP and website readiness and Erik's explicit authorization. The objective is national coverage of discoverable FAUs, not an unsupported promise that every FAU has a public contact.

1. Market owns a dated research plan: geography, school types, segmentation, competing alternatives, pricing evidence and coverage units. Verify the source universe rather than assuming which schools have active FAUs. Research KFAU as a separate future segment; KFAU signup and municipal sales are deferred.
2. Sales builds a municipality/school universe from authoritative public registries and official municipality/school pages, then identifies the corresponding FAU and its published contact route. Use public FAU sites where appropriate and retain their source links. Do not treat a principal's address as a confirmed FAU contact.
3. Store one school/FAU record with stable identifier where available, municipality, school, FAU name, official school URL, FAU URL, contact role, publicly published contact channel, exact source URL, retrieval date, verification status, duplicate/link references and next research action. Prefer institutional/shared addresses. Record named contact details only when public and necessary for the stated outreach purpose; do not collect private family information or pupil data.
4. Use explicit statuses: confirmed published FAU contact; FAU found/no public contact; school contact only; unresolved; inactive/duplicate with evidence. Never guess addresses. Preserve uncertainty and distinguish source publication dates from retrieval dates.
5. Deliver a CSV register plus methodology and a coverage report by municipality, with counts for the source universe, matched FAUs, verified contact routes, unresolved schools, duplicates and excluded records. Establish a refresh process because officeholders change. Comprehensive means transparent denominators and documented gaps.
6. Market ranks candidate pilot segments using cited evidence and stated criteria; sales prepares draft Bokmål messaging and pilot feedback materials. Legal/security review proposed contact use, minimal fields, retention and suppression handling before outreach. Research legal questions using current primary sources; this plan makes no conclusion that a particular campaign is lawful.

Suggested artifacts: `docs/research/market-map.md`, `docs/research/fau-prospects.csv`, `docs/research/fau-coverage.md`, and `docs/research/pilot-plan.md`. These are intended deliverables, not completed findings. Favro holds links and aggregate progress rather than copying the whole contact database into cards.

## Dependency order and human decisions

- Product reconciles scope; infrastructure inspection establishes actual demo integration constraints. Architecture, UX and security can investigate independently, then agree on contracts.
- Security, legal and infrastructure assess European ownership, hosting, subprocessors, contractual terms, price and delivery requirements for email/auth and other operational suppliers. European hosting alone does not establish European ownership. Report unknown ownership explicitly. Researching a supplier does not select or subscribe to it.
- Resolve the project document's administrative two-factor requirement against the later magic-link preference with Erik; do not silently remove either requirement. Resolve retention/deletion and change-set storage before treating immutable traceability as fully specified.
- Backend and frontend implementations depend on sufficiently reviewed contracts. Quality reviews each increment and verifies release conditions. Infrastructure deployment and publication require authorization for the concrete reviewed action; plans and local artifacts do not imply approval to incur costs.
- Market and prospect work can run alongside engineering. Pilot outreach depends on a working MVP/website, approved contact use and explicit authorization to send.

Escalate genuine conflicts or missing decisions on their owning card with the alternatives and evidence already prepared. Product name/domain, provider purchases, production deployment, website publication and outreach remain human decisions/actions until authorized. This specification does not invent schedules, runtime frequency, infrastructure budget, service choices or staffing commitments.
