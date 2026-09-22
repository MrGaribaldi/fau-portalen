# Frontend framework comparison for FAU

## Current decision — 11 September 2026

The editor uses Lexical (lexical.dev) in an isolated React page. The rest of the application remains Rust-rendered HTML/HTMX with focused JavaScript/TypeScript. Rust serves the compiled browser assets. The earlier Tiptap/ProseMirror preference is superseded; editor integration and the exact versioned JSON profile still require validation on #3422/#3490.

The comparisons and provisional recommendations below are historical evidence; this decision takes precedence. See #3415 and the separate editor decision #3493.

Prepared: 10 September 2026; finalized: 11 September 2026. Card: #3415. Author: Codex, project oversight.
Official sources accessed during the 10 September research session; links may change after this date.
Status: research and recommendation for review; no framework selection or implementation approval.

## Recommendation

Shortlist **SvelteKit with static output** and **Vue 3 with Vite and Vue Router**. My narrow preference is SvelteKit if one frontend project should produce both the marketing pages and authenticated app. Vue is an equally defensible choice if Rust will own public-page HTML and the frontend primarily owns the interactive application. **React with Vite** is the third serious option, particularly if a chosen editor or component library makes its integration materially easier.

This recommendation is an engineering judgment based on FAU's requirements and the official sources below. It is not a performance benchmark. SvelteKit does not win because it is assumed to be the smallest, and React is not excluded because it is assumed to require a Node server.

The decisive unresolved issue is **how newly created or updated school pages receive fresh, indexable HTML without a JavaScript server in production**. No static frontend framework solves that on its own. Choose that boundary before treating any framework as the complete solution.

This report expands the earlier `frontend-sol-handoff.md` comparison. It is a separate research artifact, with explicit alternatives, disadvantages and conditions that would change the recommendation.

## What we are choosing for

The source of product decisions is `docs/planning-decisions.md`, qualified by later explicit decisions in Favro. Technical contracts are in `docs/repo-container-contract.md`, `docs/tenant-role-history-design.md`, `docs/localisation-design.md`, `docs/url-scheme.md` and `docs/identity-and-encryption.md`. #3415 asks for bundle/runtime, editor, maintenance/licensing and accessibility comparisons, plus the important user journeys.

| Requirement | Consequence for the choice |
| --- | --- |
| Rust backend; one production image containing Rust and static frontend files | Node can run during the build. A Node SSR server, serverless functions or hosted frontend platform are not part of the accepted runtime. |
| Same-origin `/api/v1/`, `/app/` and public pages | Rust owns API authorization, HTTP status codes, redirects and serving. Client routing must not turn missing APIs/assets into successful HTML responses. |
| Parents using signup, organization switching, invitations, dated roles and an audit view | Prefer understandable forms and navigation over an elaborate dashboard framework. |
| Markdown creation/editing, autosave and exclusive leases | Editor lifecycle, focus, error recovery and stale-response handling matter more than a framework-only demo. |
| Lease ends after 15 minutes without document changes; forced release retains only server-saved content | UI must stop stale writes, distinguish saved from pending, and accurately explain discarded unsaved changes. Heartbeats must not extend inactivity. |
| Role changes apply during existing sessions; restricted administrator handover | Client state is presentation, never permission authority. A framework must make tenant-scoped state easy to clear and audit. |
| Bokmål initially, ICU catalogues and future human translations | Translation tooling must preserve the chosen catalogue format. A popular framework i18n package is not automatically ICU-compatible. |
| Public landing and school pages with meaningful titles, canonical URLs and indexing rules | Build-time HTML suits stable pages; changing school data requires a freshness strategy. Private app data must never enter public prerender output. |
| Tight MVP scope, modest infrastructure and future portability | Avoid introducing a second business backend, mandatory SaaS or two frontend stacks without a concrete payoff. |

Technical documentation stays in English. Product interface text stays in Bokmål. Meetings, tasks, live collaborative editing, OCR and payment processing are not reasons to expand the initial framework stack.

## Comparison at a glance

These assessments describe fit and work we would need to do, not measured speed or objective numerical scores. Each row is a deployment configuration, not just a library name.

| Candidate configuration | Rust-only production fit | Public HTML | Interactive application | Main advantage for FAU | Main cost for FAU |
| --- | --- | --- | --- | --- | --- |
| SvelteKit + static adapter | Yes | Built-in prerender; fresh dynamic pages need another contract | Strong fit; app can use scoped SPA fallback | One route/component project with useful navigation defaults | Must deliberately avoid Kit server features and solve public-page freshness |
| Vue 3 + Vite + Vue Router | Yes | Needs an explicit prerender or Rust-template strategy | Strong fit for forms, admin and editor state | Clear backend/frontend separation and official routing/state ecosystem | Public-page generation and navigation announcements need assembly |
| React + Vite + a selected router | Yes | Needs an explicit prerender or Rust-template strategy | Strong fit; documented editor integrations | Flexibility and direct editor/component integration paths | More independent choices for routing, data, forms and conventions |
| Astro + one Svelte or Vue app island | Yes, in static mode | Strong fit for mostly static content | Possible, but app behaves as a substantial island | Keep marketing pages mostly HTML | Two layers of routing/lifecycle/build conventions for one small product |
| Solid + Vite + Solid Router | Yes | Needs prerender or Rust templates | Fine-grained state updates suit interactive UI | Precise reactive updates without requiring a full-stack runtime | More integration validation; no demonstrated FAU benefit over the shortlist |
| Preact + Vite + preact-iso | Yes | Prerender support must be configured for the chosen route set | Suitable; React-library compatibility needs testing | Lightweight-oriented React-like option | Compatibility work may erase savings once editor and UI libraries are included |

Vite documents how a backend can serve production assets using its generated manifest. That is a concrete integration path for the Vite-based options, not a requirement to deploy the Vite development server. [Vite backend integration](https://vite.dev/guide/backend-integration).

## 1. SvelteKit with static output

**Advantages**

- Its static adapter produces HTML/assets for a conventional server, which fits Rust serving the build output. Public pages can be prerendered while an explicit fallback supports client-only routes. [Static adapter](https://svelte.dev/docs/kit/adapter-static).
- Kit supplies route navigation focus handling and announcements. These give us a useful starting point for parents using assistive technology, although document saves and FAU switches still need their own accessible feedback. [Kit accessibility](https://svelte.dev/docs/kit/accessibility).
- One frontend route tree can keep the landing-page layout, account shell and application components together. For this small team, that reduces the number of architectural conventions we must maintain. This is our assessment, not a measured productivity claim.

**Disadvantages**

- Static output cannot run request-time Kit server actions or authentication handlers. Examples that rely on them would need to be rewritten against Rust, not copied into our app.
- Client-only pages wait for JavaScript before becoming useful; Kit itself cautions about SPA performance, resilience and SEO. The public site must not become an empty application shell. [Single-page apps](https://svelte.dev/docs/kit/single-page-apps).
- The richer full-stack feature set creates a review burden: developers must remember which features our deployment intentionally does not use.
- Tiptap's Svelte integration uses the editor core with lifecycle wiring. We need to test creation, update notifications and destruction around route changes rather than assuming the wrapper handles every case. [Tiptap Svelte integration](https://tiptap.dev/docs/editor/getting-started/install/svelte).

**Choose it when:** shared public/app components and one routing project are valuable, and we accept a strict static-output discipline. Do not choose its Node adapter simply because dynamic school pages need HTML.

## 2. Vue 3 with Vite and Vue Router

**Advantages**

- Official tooling supports Vite and TypeScript-aware single-file components. Templates keep labels, validation and conditional controls close to the form structure, which I consider a good fit for FAU administration screens. [Vue tooling](https://vuejs.org/guide/scaling-up/tooling.html).
- Vue Router is officially recommended for SPAs, and Vue's state-management guidance provides a path to Pinia when shared state becomes necessary. We can start small rather than install a global store for every form. [Routing](https://vuejs.org/guide/scaling-up/routing.html), [state management](https://vuejs.org/guide/scaling-up/state-management.html).
- Tiptap provides a Vue 3 integration path. This reduces uncertainty compared with inventing an editor wrapper from scratch. [Tiptap Vue 3](https://tiptap.dev/docs/editor/getting-started/install/vue3).

**Disadvantages**

- Plain Vite/Vue does not define how to publish indexable HTML for every school route. We must choose prerendering or Rust-rendered public pages explicitly.
- Router navigation, loading/error states and focus announcements still need an application-wide convention; accessibility guidance is not automatic enforcement. [Vue accessibility](https://vuejs.org/guide/best-practices/accessibility.html).
- Broad reactive stores or watchers can obscure which operation triggered a save. We must keep editor transactions, lease changes and network commands explicit, and reset tenant-specific state on switching.
- Adding Nuxt later would be another architecture decision; it is not necessary for the Vue SPA proposed here.

**Choose it when:** the application is the main frontend responsibility and Rust/public templates can own fresh school HTML. It may be the clearest long-term separation of responsibilities for this project.

## 3. React with Vite and an explicitly selected router

**Advantages**

- React can be used with a build tool and an existing backend. It does not inherently require Next.js, a Node server or a proprietary host. The official guide explicitly describes the from-scratch option. [React from scratch](https://react.dev/learn/build-a-react-app-from-scratch).
- Tiptap provides React bindings and an integration guide. If our editor prototype or selected accessible component set is strongest here, that practical evidence could outweigh SvelteKit's routing convenience. [Tiptap React](https://tiptap.dev/docs/editor/getting-started/install/react).
- Explicit components and typed API boundaries can keep the frontend independent of provider and Rust implementation details.

**Disadvantages**

- The team must select and maintain routing, data-fetching and other conventions. React's documentation identifies this assembly work; the library alone is not a complete application framework.
- Editor subscriptions and request effects need careful ownership and cleanup. Re-rendering must not recreate the editor, duplicate saves or retain a stale tenant in a callback.
- It supplies no FAU-specific solution to public HTML, localization or administrative authorization. Adding several convenience libraries can increase upgrade and dependency-review work.

**Choose it when:** an actual editor/component trial or known maintainer expertise favors React. We have not established either advantage yet; do not substitute unverified hiring-popularity claims for that evidence.

## 4. Astro with one interactive app framework

**Advantages**

- Astro renders components to HTML by default and loads client JavaScript for explicitly interactive islands. That matches a largely static landing page, help and pricing content. [Astro islands](https://docs.astro.build/en/concepts/islands/).
- We can keep editor code out of a marketing visitor's initial page entirely and use Svelte or Vue only where interaction is needed.

**Disadvantages**

- The authenticated FAU application is continuously interactive: organization state, editor leases, autosave and dialogs do not fit naturally into many isolated content islands. One large app island is plausible, but then it has its own internal router and state conventions.
- Two layers must agree on shared CSS, navigation, error handling and translations. For this MVP, that additional work may exceed the landing-page benefit.
- Static Astro has the same school-page freshness issue as static Kit. Islands do not make request-time school HTML appear without a rendering service or regeneration process.

**Choose it when:** the public site becomes a substantial, independently maintained publishing surface. I would not introduce it solely to avoid a few scripts on the initial landing page.

## 5. Solid with Vite and Solid Router

**Advantages**

- Solid uses fine-grained reactivity, updating the parts that depend on changed state. That is conceptually suitable for save indicators, permission changes and editor toolbars. This is a documented model, not proof that our editor will run faster. [Solid overview](https://docs.solidjs.com/).
- A client build can consume the Rust API without introducing SolidStart or another runtime; Vite includes a Solid TypeScript template. [Vite getting started](https://vite.dev/guide/).

**Disadvantages**

- The editor and accessible component combinations still need concrete validation. The research here established direct Tiptap guides for React, Vue and Svelte, not an equivalent validated Solid setup; that is an evidence gap, not a claim that integration is impossible.
- Reactive tracking rules require framework-specific care. Familiar JSX syntax is not evidence that React lifecycle assumptions transfer.
- Routing, localization and public HTML still require decisions, without a demonstrated advantage for our relatively ordinary application flows.

**Choose it when:** a representative trial shows a material benefit and the team is comfortable maintaining the integrations. SolidStart's version-specific runtime requirements must not be attributed to plain Solid.

## 6. Preact with Vite and preact-iso

**Advantages**

- Its documented starter offers TypeScript and routing, and builds deployable static assets. It is worth considering when reducing client overhead is a measured priority. [Preact getting started](https://preactjs.com/guide/v10/getting-started/).
- A React-like programming model can make some existing component approaches reusable.

**Disadvantages**

- Preact documents differences from React. A compatibility layer is not a guarantee that a chosen editor, focus-management component or portal behaves identically. [Differences to React](https://preactjs.com/guide/v10/differences-to-react/).
- Any smaller core runtime can be outweighed by editor, i18n and component packages. We have no complete FAU build proving a meaningful gain.
- Public rendering and security requirements remain unchanged, while compatibility testing adds work.

**Choose it when:** a measured full-stack frontend bundle benefit survives the required editor/accessibility integrations. It is not my first choice before such evidence exists.

## Other plausible approaches

| Approach | Why it is plausible | Why it is not the first recommendation |
| --- | --- | --- |
| Next.js static export | Can produce static files; React does not force a hosted platform | Request-dependent features are restricted under static export. We would adopt a larger set of conventions while still solving dynamic public HTML separately. [Next static exports](https://nextjs.org/docs/app/guides/static-exports). |
| Angular | Provides an integrated application approach and supports static output configurations | A viable choice for an established Angular team; the structure is more than this project's current flows demonstrate a need for. This is a scope judgment, not a claim that Angular cannot be fast or accessible. [Angular rendering](https://angular.dev/guide/ssr). |
| Rust HTML templates + htmx + a dedicated editor widget | Direct route to fresh public HTML and server-owned forms; htmx enhances HTML interactions | Changes the proposed JSON/TypeScript application division and needs deliberate editor preservation during DOM updates. Worth reconsidering if backend-rendered forms become the preferred architecture. [htmx documentation](https://htmx.org/docs/). |
| Plain TypeScript and native HTML | Few framework conventions; useful for simple isolated pages | We would own routing, shared state and component lifecycles across many application states. Removing a framework does not remove that maintenance work. |

## The public-page decision applies to every candidate

There are three distinct rendering needs:

1. **Stable marketing pages:** generate HTML at build time and serve it from Rust. Keep editor/authentication packages out of these pages where possible.
2. **Authenticated application:** serve an application shell, fetch authorized data from Rust, and make loading/failure states accessible. Search indexing is not a reason to prerender private data.
3. **School pages that change after deployment:** choose one of the following explicitly.

| Strategy | Benefit | Cost and required safeguards |
| --- | --- | --- |
| Prerender a public-data snapshot and regenerate on changes | Stays within the static artifact contract; no new serving runtime | Define freshness and regeneration ownership; prove stale claimed/unclaimed metadata is handled; never rebuild from private member/document data. |
| Rust renders public HTML using escaped templates and the asset manifest | Fresh HTML, canonical metadata and true HTTP errors without Node | Adds a Rust presentation layer and a shared design/copy contract; architecture must own it. Vue or React becomes more attractive if Kit's public rendering is no longer needed. |
| Introduce a JavaScript SSR server | Framework-native request-time rendering | Explicit amendment to the accepted runtime, additional patching/process monitoring and deployment integration. This report does not recommend or authorize that change. |

A client changing the page title after load is not equivalent to a tested public HTML strategy. Rust must retain real 404/410/redirect handling; SPA fallback is scoped to intended app navigation and never to missing `/api/v1` or asset paths. School canonicalization is server-owned even if client routing repeats it for navigation.

## Editor choice is a separate, potentially larger decision

FAU needs Markdown documents and exclusive editing, not Google-Docs-style concurrent editing. Do not add CRDT infrastructure or a paid collaboration service just because an editor advertises it.

| Editor approach | Advantages for FAU | Disadvantages and validation needed |
| --- | --- | --- |
| Plain text area plus safe Markdown preview | Preserves Markdown directly; few editor lifecycle dependencies; baseline native input behavior | Parents may need formatting buttons and help; preview must be sanitized; switching between source and preview needs good focus behavior. |
| CodeMirror-based source editor | Structured text-editing foundation; independent editor state can be integrated with a component framework | It is a code/text editor, not automatically an approachable word processor. Validate touch selection, VoiceOver, keyboard shortcuts and Markdown affordances. [CodeMirror project](https://github.com/codemirror/dev). |
| Tiptap-based rich editor | Directly documented React/Vue integrations and a Svelte integration route; potentially easier formatting controls | Prove Markdown round-trip behavior and supported syntax. Its Markdown documentation currently labels the feature beta; do not assume lossless conversion. [Tiptap Markdown](https://tiptap.dev/docs/editor/markdown). |

Start with actual examples: paragraphs, headings, nested lists, links, quotes and any table syntax we choose to support. Compare source before/after opening and saving. Preserve document semantics rather than silently discarding unsupported content. The accepted Markdown subset must be agreed with the backend.

The wrapper must release subscriptions on navigation, never recreate the editor on each keystroke, stop writes after lease generation changes, and never report a failed save as successful. Role loss and forced release have different consequences from a recoverable network interruption; test them separately.

## Accessibility, localization and security

No candidate makes the finished product accessible or secure by itself. SvelteKit's route support is a useful advantage; Vue's guidance is useful guidance. The deciding evidence is our actual flows with keyboard, touch, zoom and a screen reader.

Use WCAG 2.2 AA as the proposed engineering target, not a claim of legal compliance. Test visible focus, form error associations, route/save announcements, responsive reflow, accessible authentication and target sizing. Include iPhone Safari with VoiceOver, as well as desktop keyboard and screen-reader use. [WCAG 2.2](https://www.w3.org/TR/WCAG22/).

All shortlisted options can call a framework-independent message formatter. ICU syntax includes plural/select messages; the chosen binding must be demonstrated against our catalogue rather than replaced with sentence concatenation. Keep `nb-NO` complete, delay loading future locales, use `Intl` for formatting, and create no machine-written Nynorsk catalogue. [ICU message syntax](https://formatjs.github.io/docs/core-concepts/icu-syntax/).

Security controls derive from the FAU design and must be tested regardless of framework:

- Rust authorizes every operation; hidden buttons and router guards are not authorization.
- Cancel or ignore old-tenant requests, key caches by tenant/resource, and clear protected views on logout or loss of access.
- Use the agreed cookie/session and CSRF contract. Do not store provider tokens in browser persistence or implement background authentication refresh.
- Treat Markdown/raw HTML and download content as untrusted. Raw-HTML escape hatches require a reviewed rendering policy.
- Never prerender member names, invitation secrets or document content into a public build. Use synthetic test data.
- Keep fonts, scripts and packages in our served assets unless a separate supplier decision permits an external request. A downloadable open-source library and a SaaS receiving member data are different dependencies under the project policy.

## Bundle size, running costs and licensing

There are **no measured FAU bundle sizes or performance results in this report**. Comparing a framework's tiny hello-world runtime with another candidate's full application would be misleading. The relevant costs include the editor, router, ICU parser/catalogue, icons, CSS, validation and accessibility components.

All six principal configurations can avoid an additional JavaScript production server. Their infrastructure difference is therefore primarily bytes served and any public-page regeneration work, not an assumed extra VM. Browser parsing and interaction latency matter to users even when server cost is small. Development and upgrade effort may outweigh hosting differences at MVP scale; no monetary estimate is asserted.

| Core project | Licence inspected | Scope of this finding |
| --- | --- | --- |
| SvelteKit | MIT | [Project licence](https://github.com/sveltejs/kit/blob/main/LICENSE); does not certify every Svelte/adapter dependency |
| Vue core | MIT | [Project licence](https://github.com/vuejs/core/blob/main/LICENSE); router and plugins need their own inventory |
| React | MIT | [Project licence](https://github.com/facebook/react/blob/main/LICENSE); component libraries are separate |
| Astro | MIT | [Project licence](https://github.com/withastro/astro/blob/main/LICENSE); integrations are separate |
| Solid | MIT | [Project licence](https://github.com/solidjs/solid/blob/main/LICENSE); router and editor packages are separate |
| Tiptap open-source editor repository | MIT | [Repository licence](https://github.com/ueberdosis/tiptap/blob/main/LICENSE.md); commercial extensions and services must be checked separately |

These are checks of current project licence files, not a legal opinion or a completed dependency audit. Preact and the secondary alternatives have not received a full licence review here. Before implementation locks a choice, record exact versions, transitive licences, release/security support policy, Node build version and lockfile. No claim is made that any package is vulnerability-free.

Maintainability for FAU means selecting a small, documented package set, keeping generated API types aligned with Rust, isolating the editor behind a small interface, and scheduling dependency review. Framework syntax familiarity among future maintainers is still unknown; no hiring statistics or market-share estimates were collected.

## Evidence needed to finalize the choice

The documentation comparison is complete enough to shortlist; it is not an implementation bake-off. A bounded SvelteKit-versus-Vue trial should use identical scope and the same editor and message catalogue so the result is comparable.

| Check | Common test for both candidates | Decision consequence |
| --- | --- | --- |
| Production artifact | Build and serve public HTML plus `/app/` through a minimal Rust contract; deep-link reload; missing API/asset errors | Reject configurations requiring an unapproved runtime or global HTML fallback |
| Bundle | Report uncompressed, gzip and Brotli JS/CSS per landing, app shell and lazy editor route, plus package versions | Judge complete route cost; never pick a winner using core-runtime marketing numbers |
| Forms/navigation | Signup errors, organization switch and expired session with synthetic responses | Prefer the implementation with fewer special cases and clearer focus/error handling |
| Editor | Same Markdown samples; selection, undo, autosave acknowledgement, lost lease, reconnect and mount/unmount | Reject corruption, stale saves or inaccessible essential editing |
| Device behavior | iPhone Safari/VoiceOver, keyboard-only desktop and throttled cold loads | Reject essential interactions that fail; document versions and conditions |
| i18n | ICU plural/select fixtures, missing-key checks and Bokmål export/import | Reject a binding that changes the catalogue contract or exposes keys to users |
| Security | Old-tenant response arriving after switch; logout cleanup; unsafe preview; built-output scan | Reject cross-tenant display or unsafe persistence/rendering |
| Maintenance | Package/licence list, upgrade instructions and documented API/editor boundaries | Prefer the smaller reviewed integration burden, not merely fewer lines of code |

Do not invent a delivery date or a performance budget from this report. Agree numerical budgets before scoring a trial, record actual measurements, and distinguish automated checks from manual verification.

## Decision to record on #3415

My proposed decision is: **trial SvelteKit static output against Vue 3/Vite, with SvelteKit as the provisional default for a unified website/app project**. Keep React/Vite available if editor evidence favors it. Resolve public school HTML ownership alongside the trial.

Change the preference to Vue if Rust-rendered public pages are selected and Kit's integrated public routing offers little benefit. Change it to React if the chosen editor/component combination demonstrably reduces integration risk. Choose Astro only if a separate content-heavy public site becomes worth maintaining. Choose Solid or Preact only when a representative trial demonstrates a benefit that justifies their additional integration validation.

The report does not request adoption of a paid service, change Terraform, amend the runtime contract, or mark #3415 Done. It supplies the comparative evidence and a concrete route to a reviewable selection.

## Current recommendation

Implement Lexical in the React editor page and retain HTML/HTMX elsewhere. Validate accessibility, editor lifecycle, autosave/leases, schema round trips, Rust rendering, bundle size and package licences through #3422/#3490.
