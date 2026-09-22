# Frontend requirements and framework evaluation

## Current decision — 11 September 2026

The editor uses Lexical (lexical.dev) in an isolated React page. The rest of the application remains Rust-rendered HTML/HTMX with focused JavaScript/TypeScript. Rust serves the compiled browser assets. The earlier Tiptap/ProseMirror preference is superseded; editor integration and the exact versioned JSON profile still require validation on #3422/#3490.

The comparisons and provisional recommendations below are historical evidence; this decision takes precedence. See #3415 and the separate editor decision #3493.

Date: 10 September 2026. Owner for later implementation: `frontend` role. This document is a
requirements and decision handoff. It does not choose a framework, claim a Favro card, authorize
application code, or authorize deployment.

## Status and source precedence

Later explicit decisions override earlier proposals. The working order for this handoff is:

1. Erik's dated decisions in `docs/planning-decisions.md`;
2. accepted contracts and their recorded conditions;
3. current card descriptions and comments in Favro;
4. proposed ADR or design text;
5. the broader goals in `prosjektgrunnlag.md`.

Favro access was restored after initial preparation. Card #3415 was read using its internal
cardCommonId, `6a6e8ff6087ca28f87da5802`; numeric display IDs are not CLI identifiers.
Its current acceptance requires a framework comparison covering bundle/runtime needs, editor,
maintenance/licensing and accessibility, plus landing/signup/FAU-switch/admin/editor/audit flows.
Other card discussions and approvals still require reconciliation before implementation.

Two repository inconsistencies are implementation gates rather than frontend decisions:

- `docs/url-scheme.md` calls `/s/<school-uuid>` permanent and says it never redirects, but its
  merger table later prescribes a redirect. The API/routing owner must settle the invariant and
  tests before frontend routing encodes either behavior.
- `docs/identity-and-encryption.md` equates repeated step-up authentication with the original
  administrative two-factor requirement. Repeating an email factor does not itself establish two
  independent factors. Security must define the required assurance and the backend response
  contract before privileged UI is enabled.

The identity document's statement that deleting a wrapped tenant key makes every backup copy
unrecoverable is also unproven while a backup may contain that wrapped key and a usable master-key
path remains. Frontend and website copy must not promise cryptographic erasure until security and
infrastructure provide a reviewed key/backup lifecycle and restore test.

## Scope boundary

The frontend workstream will own files under `frontend/` only once its cards are inspected and
claimed. Expected later contents are application source, public pages, Bokmål catalogue, styles,
frontend tests, package manifest and lockfile, and build configuration. There is no `frontend/`
directory at the time of this handoff.

The frontend workstream does not own Rust/API code, schemas, migrations, shared API generation,
Compose, Kubernetes, Terraform, agent images, provider configuration, authentication policy,
security headers at the ingress, product naming, domains, publication, or deployment. A necessary
change outside `frontend/` is handed to its owner instead of being edited incidentally.

MVP frontend scope, after prerequisites settle:

- public landing and product pages, school browsing and school/FAU entry pages;
- passwordless sign-in entry and provider return/error states;
- self-service FAU creation with FAU name, school name and leader email;
- FAU selection and a clearly announced switch when a permitted resource link changes active FAU;
- initial school organization setup;
- member invitations and dated roles, with privileged actions gated by backend-confirmed step-up;
- Markdown document list, creation and accessible editing with save, stale-lock and conflict states;
- a role-gated audit view;
- Bokmål interface strings through a translation-ready catalogue.

Version comparison/restoration UI, broad import/OCR/spreadsheet features, tasks, meetings, public
minutes, search and export appear in the wider product document but are outside the narrowed MVP
recorded in `docs/planning-decisions.md` unless owning cards explicitly restore them to the first
increment. Payment is later. Nynorsk text and a language switcher are not shipped now.

## Requirements matrix

| Area | Confirmed repository requirement | Frontend consequence | Required upstream contract/evidence |
|---|---|---|---|
| Runtime boundary | Rust backend and lightweight TypeScript frontend; one production image and same origin are accepted implementation bases, conditional on later card details | Build emits static assets served by Rust through the agreed build contract; no production origin or secret is compiled into browser code | #3422 plus backend/build owner defines artifact directory, fallback behavior, cache headers and generated API type workflow |
| Routes | `/`, public product routes, `/fau/...`, `/s/<uuid>`, `/bli-med/<uuid>`, `/app/`, `/app/dokumenter/<uuid>`, `/app/audit`, `/api/v1/...` and `/assets/...` are reserved | File routing must preserve the map, lowercase/slash canonicalization and future outer public locale prefix | Resolve the `/s/<uuid>` merger contradiction; API defines status/error codes and redirect ownership |
| Tenant isolation | Server authorizes every operation; an inaccessible cross-tenant resource returns 404 | UI never treats active-FAU state as authority and renders the same not-found experience for absent and unauthorized resources | Backend contract for typed 404 and permitted resource-driven tenant switch |
| Active FAU | Opening an authorized document may change the active FAU | Announce the switch visibly and to assistive technology before the user edits; all subsequent context identifies the active FAU | Atomic backend response/session semantics and stable tenant display fields |
| Identity | Zitadel Cloud Swiss region is the current time-boxed provider; OIDC Authorization Code with PKCE; authorization remains internal | Frontend initiates the abstract passwordless flow and handles states, but does not consume provider roles/groups as permissions or persist bearer tokens in browser storage | Auth/backend callback and session contract, issuer mapping, CSRF/state/nonce behavior and provider-neutral error codes |
| Passwordless mechanism | Requirement is passwordless proof by email; link versus one-time code remains provider-specific | UI copy and component boundaries must accommodate both mechanisms without exposing provider assumptions | #3415/auth owner settles the initial mechanism and resend/expiry/rate-limit responses |
| Sessions | Ordinary sessions last 30 days; shared-device option disables the long session; tokens refresh only after real interaction | No background refresh, idle-tab keepalive, or service-worker renewal; expose the shared-device choice in the agreed flow | Backend owns cookies, expiry, rotation, revocation and interaction-triggered refresh endpoint semantics |
| Privileged actions | Invite, grant/revoke roles and recovery initiation require stronger, recent authentication | Disable submission until the server confirms sufficient assurance; handle expired assurance and return focus to the initiating control | Security resolves MFA/step-up assurance; backend supplies typed `reauth_required` behavior and continuation semantics |
| Membership and roles | Membership and roles are dated; expired roles lose access immediately even in a live session | Never cache authorization as durable UI state; failed writes refresh capabilities and explain the change without revealing protected data | API exposes current capabilities and stable denial codes per operation |
| Signup | Self-service creates an FAU immediately from FAU name, school name and leader email and notifies Erik | Accessible, correction-friendly form; a prefilled school is a suggestion and grants no access | #3422/API schema, validation/error codes, rate limits and school verification states |
| Documents | MVP creates/edits Markdown; change sets are stored from the start; one active editor, 15-minute inactivity lock expiry; administrator handover access lasts six months and permits only inviting replacements, assigning roles and granting administrator access | Editor provides semantic formatting controls, save state, lock owner/expiry, loss-of-lock and stale-write conflict recovery without silent overwrite | API contract for document/version identifiers, lock token/version, renewal, stale precondition response, autosave idempotency and error codes |
| Audit | Important events are role-gated and immutable in the application model | Render stable event codes through catalogue strings; unknown codes get a safe generic fallback; do not infer access from hidden navigation | #3439 catalogue contract and audit API pagination, actor/time/tenant metadata and authorization |
| Localisation | `nb-NO` is default/fallback; Nynorsk is design-only; no `nn-NO.json` stub; stable codes map to localized text | All product text lives in a complete Bokmål ICU MessageFormat catalogue; no concatenated sentences; public route structure keeps outer `/nn/` cheap later; workspace locale eventually comes from account | #3439 review plus framework-specific i18n package decision and Bokmål export/import round-trip |
| Public pages and indexing | School pages are real public pages; verified/claimed state controls canonical/indexing; unclaimed/unverified pages are `noindex` | Prerender suitable pages or use an agreed Rust-rendered HTML contract, with unique titles, canonical URL and robots metadata from typed verification state | #3423 receives page data, canonical URL and indexing state from API; product supplies approved content/name |
| Files and Markdown safety | Uploaded/fetched documents are untrusted; the later encrypted-storage design specifies backend-proxied downloads; HTML/SVG are unsafe inputs | Never inline untrusted uploads; rendered Markdown uses a reviewed allowlist/sanitizer and treats raw HTML as disabled by default; external links are safe and visibly identifiable | Backend supplies the authorized download contract and content metadata; security approves Markdown rendering policy and CSP |
| Privacy claims | Encryption protects against a leak, not the operator; erasure-via-key-destruction is not yet demonstrated across backups | Website and settings copy say only what evidence supports; no “we cannot see your data” or guaranteed backup erasure claim | Legal/security-approved copy and tested key/backup lifecycle |

## Confirmed decisions, proposals and open gates

| Classification | Item |
|---|---|
| Confirmed in repository decision history | TypeScript and a lightweight framework; Rust API; PostgreSQL; same-origin app boundary; Bokmål source UI; translation-ready design without Nynorsk text; stable error/event codes; server-side authorization; Markdown editing; dated roles; explicit lock/save states; Zitadel as a time-boxed identity provider; OIDC code flow with PKCE; provider-neutral authorization model |
| Recommended here, requiring review | SvelteKit with strict TypeScript and static output compatible with the Rust runtime; dynamic public-page rendering remains an architecture dependency; WCAG 2.2 AA as the engineering acceptance target; progressive enhancement for ordinary forms; a reviewed Markdown editor library rather than a custom editor engine |
| Proposed elsewhere, not silently accepted here | Exact i18n library and ICU implementation; framework/API type generator; raw Markdown renderer/sanitizer; editor library; CSP; exact canonical redirects; runtime artifact handoff; exact passwordless mechanism |
| Blocking before app code | Inspect and claim owning cards; settle framework and API/build contracts; resolve `/s/` redirect semantics; define auth/session/CSRF contract; define MFA/step-up assurance; define document lock/autosave preconditions; obtain UX flow/copy acceptance criteria |
| Blocking before public claims/release | Product name/domain and approved Bokmål copy; indexing-state API; legal/privacy review; demonstrated deletion/backup claims; accessibility and security review evidence |

## Favro dependencies

Live state is unverified. These are repository-derived relationships to reconcile, not claims about
the cards' present lanes or approvals.

| Card | Expected ownership/input | Frontend dependency and handoff condition |
|---|---|---|
| #3415 | UX and universal design; framework evaluation appears routed here in repository documents | Must provide reviewed journeys, responsive states, component behavior, Bokmål copy approach, accessibility criteria, and framework decision or explicit delegation. Frontend must not claim it solely because this handoff recommends a candidate. |
| #3422 | Authenticated application frontend | Expected owning implementation card for `/app/`, signup, switching, setup, invitations, editor/locks and audit. Claim before creating `frontend/`; record API/auth dependencies and reviewer. |
| #3423 | Landing/public website | Provides approved page scope, content, product identity constraints, public school page/indexing behavior and SEO acceptance. It may be independently incremented once identity-free copy and routes are agreed. |
| #3439 | Localisation design | Repository says it is non-blocking after stable codes and design-only Nynorsk were decided, but implementation still needs a framework-specific ICU library and complete `nb-NO` catalogue. No agent-authored Nynorsk or placeholder catalogue. |

Related upstream dependencies include tenant and role authorization (#3412), auth/session (#3414 and
#3417 in repository references), generated API contract/build boundary (#3416), document behavior
and ingest (#3419), storage/download safety (#3435), and the school register/routing source
(#3441). Product should confirm the current mapping after live-card inspection.

## Lightweight TypeScript framework evaluation

Research was checked against official project documentation on 10 September 2026. Package versions
and lockfiles are deliberately absent because no dependency install is authorized yet.

| Candidate | Fit | Material drawback for FAU | Assessment |
|---|---|---|---|
| SvelteKit | One router can prerender marketing routes, SSR dynamic public routes and hydrate the authenticated app. Svelte compiles components, SvelteKit includes navigation accessibility behavior, and the official Node adapter creates a standalone Node server. | More browser JavaScript than Astro on content-only pages; route announcements rely on correct unique titles; framework checks do not prove application accessibility. | **Candidate for review with static output.** Node SSR conflicts with the accepted Rust-only runtime and is not recommended without an explicit architecture change. Bundle size and editor fit are not measured yet. |
| Astro with Svelte islands | Static HTML by default and JavaScript only for explicit islands; strong fit for landing and public content, built-in TypeScript, SSR through an official Node adapter. | Signup, organization setup, editor, locks and audit are highly interactive and may converge into a large island or separate SPA. That creates two routing/state/component models and complicates authenticated navigation and localisation. | Strong alternative if product deliberately splits public site and app. Do not adopt accidentally through landing-page-first work. |
| SolidStart v2 | Fine-grained reactivity, SSR by default, file routing and strict server/client boundaries; JSON serialization can avoid `unsafe-eval`. | Current v2 docs require Node 24+, routing is selectable, v2 is recent, and the project would assume a smaller ecosystem for editor/a11y integrations without a demonstrated payoff. | Keep as an alternative, but it has higher framework and tooling risk for the MVP. |

Recommendation: evaluate **SvelteKit with static output** for both public pages and the
application. The accepted production contract contains Rust and static frontend assets; it does
not include a Node server. Node is a build-time tool only. The Node adapter below was researched
as an alternative, not an approved deployment choice. Dynamic school pages need an explicit
prerender/rebuild or Rust HTML-rendering contract so indexing, canonical URLs and data freshness
work without silently adding a second runtime. Keep API calls under `/api/v1`.

The recommendation remains preliminary: no bundle benchmark, editor integration trial, pinned
package licence inventory or production build has been completed. These are remaining #3415
acceptance items, not verified advantages of a selected framework.

This recommendation is based on:

- [SvelteKit introduction](https://svelte.dev/docs/kit/introduction): integrated application
  framework and router;
- [SvelteKit page options](https://svelte.dev/docs/kit/page-options): per-route prerender, SSR and
  client rendering, including a documented mixed marketing/dynamic/admin model;
- [SvelteKit accessibility](https://svelte.dev/docs/kit/accessibility): compile-time checks, route
  announcements, focus handling and the requirement for unique titles;
- [SvelteKit Node adapter](https://svelte.dev/docs/kit/adapter-node): official standalone Node
  output;
- [Astro islands](https://docs.astro.build/en/concepts/islands/),
  [TypeScript](https://docs.astro.build/en/guides/typescript/) and
  [on-demand rendering](https://docs.astro.build/en/guides/on-demand-rendering/): HTML-first
  behavior, explicit client hydration and official SSR adapters;
- [SolidStart v2 overview](https://docs.solidjs.com/solid-start/v2),
  [getting started](https://docs.solidjs.com/solid-start/v2/getting-started) and
  [configuration](https://docs.solidjs.com/solid-start/v2/reference/config/solid-start): current
  v2 runtime, Node requirement, SSR and environment/CSP boundaries.

Approval of a framework must record the exact major version, supported Node line, package manager,
lockfile, adapter, update policy, licence check, build artifact and API type-generation path.

## Security acceptance criteria for the first executable increment

These criteria are frontend-verifiable parts of larger controls. Passing them does not establish
system security or GDPR compliance.

1. Browser code contains no secret, provider management credential, private endpoint, or
   environment-specific production origin. A built-asset scan finds none of the configured secret
   names or test secret values.
2. Authentication uses a backend/session boundary. No access token, refresh token or OIDC code is
   stored in `localStorage`, `sessionStorage`, IndexedDB or a service-worker cache. Provider roles,
   groups and tenant claims never authorize or reveal a control.
3. Every state-changing request uses the agreed same-origin CSRF control and sends no mutation on
   page load. Login return state/nonce/PKCE failures display a generic, localized error and do not
   leak query values, codes or tokens into logs or rendered HTML.
4. The app performs no timer-, visibility-, service-worker- or idle-tab-based token refresh. A
   network test with an idle tab records zero auth refresh requests; refresh occurs only through
   the backend after an actual user interaction under the agreed contract.
5. Privileged controls submit only after a backend-confirmed assurance state. A stale/missing state
   produces the typed reauthentication path, then resumes or safely restarts the intended action;
   the frontend never upgrades the session itself. Release waits for security to resolve whether
   this requires an independent second factor.
6. Capabilities are refreshed after every authorization failure. Removing or expiring a role in a
   live session causes the next protected operation to fail closed without briefly committing a
   client-only change.
7. Missing and inaccessible cross-tenant document URLs render indistinguishable 404 pages. No
   title, FAU name, timing-dependent preview, analytics event, error detail or cached content
   confirms that the inaccessible resource exists.
8. A permitted deep link that changes active FAU shows and announces the new FAU before edit
   controls activate. All mutation requests use resource identifiers and backend authorization;
   a client `activeFauId` is never accepted as proof of access.
9. Markdown preview disables raw HTML by default and passes an agreed sanitizer/allowlist. Tests
   cover script/event attributes, `javascript:` and unsafe `data:` URLs, SVG/foreign content,
   malformed links and reverse-tabnabbing. Untrusted uploads are downloaded, never embedded or
   rendered in the application origin.
10. The deployed response policy supports a CSP without `unsafe-eval` and without inline scripts
    that lack the chosen nonce/hash strategy. This is tested against production output, not only
    development mode. Security owns the final header policy.
11. Autosave sends an idempotency key and the last observed version/lock precondition. A stale or
    lost lock can never be reported as saved; forced release keeps only the latest server-saved version and discards unsaved changes, as decided. Ordinary transport failures need a separate recovery flow that never retries with a revoked lease or persists protected content after logout.
12. Errors shown to users come from stable bounded codes, while server details and submitted
    values remain out of DOM diagnostics and analytics. No real customer or pupil data appears in
    fixtures, snapshots, logs or test recordings.

## Accessibility acceptance criteria

Proposed engineering target: WCAG 2.2 level AA, pending UX/legal confirmation of the formal release
baseline. Automated checks supplement keyboard and assistive-technology review.

1. Every route has one descriptive `h1`, a unique descriptive `<title>`, correct `lang="nb-NO"`,
   semantic landmarks and a keyboard-visible skip link to main content.
2. All functions work with keyboard alone in a logical order, including navigation, dialogs,
   organization switcher, formatting toolbar and conflict recovery. Focus is always visible and is
   neither obscured by sticky UI nor trapped outside an open modal.
3. Client navigation moves focus according to an agreed pattern and announces the new route.
   Active-FAU changes, save results, lock acquisition/loss and validation summaries are announced
   through appropriately scoped live regions without repeating on every keystroke.
4. Forms have persistent visible labels, programmatic names, instructions before input, correct
   autocomplete/input types, field-level error association and an error summary that links to and
   focuses the first invalid field. Errors are not identified by color alone.
5. Passwordless link/code expiry, resend throttling, session expiry and reauthentication explain
   what happened and provide a keyboard-operable recovery action without requiring memory of
   hidden values.
6. Text and controls meet WCAG AA contrast; focus indicators have at least a 3:1 contrast change;
   information remains usable at 200% text zoom and 400% browser zoom/reflow at 320 CSS px without
   two-dimensional scrolling except for genuinely tabular/editor content.
7. Pointer targets meet the WCAG 2.2 AA 24-by-24 CSS-pixel minimum or the spacing exception.
   Dragging is never the only way to reorder or move items.
8. Motion respects `prefers-reduced-motion`; no essential status depends on animation, hover,
   sound, shape, position or color alone. There is no flashing content.
9. The Markdown editor exposes semantic toolbar button names and pressed states, preserves a
   usable plain-text editing path, does not intercept standard browser/screen-reader shortcuts,
   and provides accessible help for nonstandard shortcuts. Preview has valid heading/list/table
   semantics.
10. Lock and conflict dialogs name the document and consequence, focus their heading on open,
    explain that forced release discards unsaved changes, provide only actions permitted by current authorization, and return focus to
    a meaningful control. Time limits warn users and allow extension unless the lock's security or
    concurrency rule requires expiry.
11. Responsive content order matches DOM and reading order. Visual CSS reordering never changes
    meaning; touch, mouse, keyboard and screen-reader users receive the same information and
    actions.
12. Acceptance evidence includes: framework compile-time a11y checks with zero unreviewed warnings;
    automated WCAG checks on each representative route/state; keyboard-only review at 320/1280 CSS
    px; screen-reader review of navigation, signup, switching, editor/save/lock and errors; contrast
    measurements; and documented outcomes for any exception. A passing automated scan alone is
    insufficient.

## Recommended first reviewable increment

After the cards are inspected/claimed and framework/API prerequisites are accepted, create one
small vertical shell under `frontend/`:

- the chosen framework scaffold with strict TypeScript, pinned lockfile and production build;
- shared semantic layout, skip link, focus style, error boundary and complete Bokmål catalogue for
  this increment;
- `/` as a content skeleton using explicitly approved neutral copy or marked placeholders;
- `/app/` as a server-backed session-state shell showing signed-out, signed-in/no-FAU, FAU chooser,
  loading, 404 and generic error states against a mocked/generated typed contract;
- no document editor, real provider integration, mutations, customer data, production hostname or
  deploy configuration.

Review evidence should be a reproducible typecheck, production build, focused route/component
tests, built-asset secret scan, keyboard walkthrough and automated accessibility report. This
increment proves the framework, build artifact, route split, translation mechanism, API typing,
session boundary and accessible navigation before the team commits to the riskier editor and auth
flows.

## Exact next step

Product uses the restored private `FAVRO_ENV_FILE`, runs
`favro check`, and reconciles #3415, #3422, #3423 and #3439 including comments, dependencies,
assignment, acceptance and review state. Product then routes the SvelteKit recommendation for a
recorded accept/reject decision and assigns/claims the single owning implementation card. Frontend
starts the shell increment only after #3422/#3416 provide the build/API contract and security has
provided the session/CSRF and step-up-assurance contracts.

## Current recommendation

Implement Lexical in the React editor page and retain HTML/HTMX elsewhere. Validate accessibility, editor lifecycle, autosave/leases, schema round trips, Rust rendering, bundle size and package licences through #3422/#3490.
