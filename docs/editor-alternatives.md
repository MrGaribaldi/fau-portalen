# Document editor alternatives for FAU

## Current decision — 11 September 2026

The editor uses Lexical (lexical.dev) in an isolated React page. The rest of the application remains Rust-rendered HTML/HTMX with focused JavaScript/TypeScript. Rust serves the compiled browser assets. The earlier Tiptap/ProseMirror preference is superseded; editor integration and the exact versioned JSON profile still require validation on #3422/#3490.

The comparisons and provisional recommendations below are historical evidence; this decision takes precedence. See #3415 and the separate editor decision #3493.

Date: 11 September 2026. Decision card: #3493. Owner: 🧭 Produkt og prosjektledelse.
Status: research for human review, not package selection or subscription approval.

Subsequent user preference: Lexical is now the leading candidate because its
playground matches the desired editing experience. Erik accepts an isolated,
compiled browser-only React editor within the Rust-rendered HTML/htmx site as an
option for evaluation. No Node production backend is proposed. This supersedes
the provisional candidate ordering below; compare actual feature reuse and
maintenance effort before final adoption. See #3493 and planning-decisions.md.

## Decision and scope

Choose the text editor independently of the website framework. Erik's preferred direction is Rust-rendered HTML, htmx for partial server interactions, and focused JavaScript/TypeScript. A self-contained editor can run in that architecture. Future presentations and spreadsheets need their own evaluation; none of these text editors establishes a complete office suite.

The earlier Tiptap preference was provisional. This comparison reopens the editor choice rather than treating that preference as evidence. The latest structured-content decision supersedes the older Markdown-only comparison attached to #3415. Selecting a different document model requires an explicit amendment on #3490; JSON formats from different editors are not interchangeable.

Required baseline: approachable formatting, headings, lists/checklists, links, images/attachments, tables, keyboard and touch use, Bokmål UI, server-acknowledged autosave and exclusive editing. Preserve encrypted storage, session-grouped snapshots, tenant authorization and safe publication. Anchored comments belong to #3491; DOCX fidelity and timing belong to #3433. Simultaneous collaboration and commercial high-fidelity conversion remain deferred in the current record.

## Alternatives: advantages and disadvantages

The effort ratings below are product/engineering judgments, not measured estimates. No candidate has been prototyped against FAU acceptance tests in this comparison. Published feature availability is not evidence of accessibility or conversion fidelity in our application.

| Option | Advantages | Disadvantages and work FAU owns | Cost and integration implications |
| --- | --- | --- | --- |
| **Tiptap open-source core** | Framework-independent; extension API over ProseMirror; aligns with the provisional schema direction; supports a deliberately limited editor | Headless: we assemble toolbar, styling and behavior. Advanced commercial features are not included merely because the core is free. Custom comments, persistence and conversion remain work | MIT core, no core subscription. Embed with vanilla JS. Audit each selected extension separately; moderate baseline integration effort |
| **Tiptap paid features/platform** | Packaged comments, history and conversion capabilities may reduce custom feature development | Subscription and feature/usage limits; deployment and data-processing terms need checking. Vendor history is not automatically FAU's encrypted audit history. Cloud processing may conflict with the intended data boundary | Compare the exact proposed package and hosting configuration; do not assume self-hosting removes commercial fees. No purchase recommendation without a scoped quote |
| **ProseMirror directly** | Detailed control of schema, transactions and position mapping; avoids Tiptap's wrapper; fits a custom embedded editor | A toolkit rather than finished UI. More responsibility for commands, menus, plugins and accessible behavior. Does not make comments or Word conversion free to implement | Open-source foundation; verify licences of the chosen modules. High assembly effort; useful when required control outweighs that maintenance |
| **Lexical** | MIT; dependency-free core, vanilla JS entry point and serializable structured state; credible independent alternative to the ProseMirror family | We assemble the editor UI and feature integrations. Its JSON differs from ProseMirror; schema, rendering and import contracts need revision. React examples must not be mistaken for a requirement to use React | No core subscription. Moderate-to-high integration effort pending a vanilla JS trial |
| **Quill** | BSD-licensed; toolbar/themes and a straightforward API; attractive for a modest formatted-text editor | Delta is a different content model. Required table behavior needs a specific validated solution; comments and office conversion are not established by its basic feature list | No core subscription. Potentially simpler baseline, but richer document requirements can erase that advantage |
| **CKEditor 5** | More packaged word-processor UI; modular architecture; commercial advanced-feature route offers an alternative to building everything | GPL/commercial licensing decision; premium coverage and hosting must be priced. Its internal model and persisted output need an explicit mapping to our structured schema | Credible buy-versus-build comparator. Less UI assembly may be offset by licensing and integration cost; package quote required |
| **TinyMCE** | Conventional document-editing UI; embeddable JavaScript; extensive documented plugin selection | GPL/commercial choice; comments and Word-related features must be checked against premium plans. HTML-oriented persistence requires adaptation rather than storing unrestricted HTML | Credible packaged-editor comparator; evaluate self-hosted terms, premium plugins and conversion services separately |
| **BlockNote** | More assembled block-editor experience; structured blocks; potentially less toolbar/UI work | Different interaction style from a conventional word processor. Current getting-started path uses React UI packages, which adds an editor island integration choice. Core and XL packages have different licences | Core MPL 2.0; XL terms may require a commercial licence for the intended use. Validate the exact UI integration rather than assuming framework-free embedding |

Sources for these distinctions, accessed 11 September 2026:

- Tiptap: [vanilla JS integration](https://tiptap.dev/docs/editor/getting-started/install/vanilla-javascript), [feature comparison](https://tiptap.dev/feature-comparison), [pricing and deployment questions](https://tiptap.dev/pricing).
- ProseMirror: [guide and document model](https://prosemirror.net/docs/guide/), [examples](https://prosemirror.net/examples/).
- Lexical: [introduction and state model](https://lexical.dev/docs/intro), [MIT licence](https://github.com/facebook/lexical/blob/main/LICENSE).
- Quill: [design and API](https://quilljs.com/docs/why-quill), [documented formats and licence](https://quilljs.com/docs/formats).
- CKEditor: [editor architecture](https://ckeditor.com/ckeditor-5/), [licensing options](https://ckeditor.com/legal/ckeditor-licensing-options/), [pricing](https://ckeditor.com/pricing/).
- TinyMCE: [licence configuration](https://www.tiny.cloud/docs/tinymce/latest/license-key/), [open-source and premium plugin catalogue](https://www.tiny.cloud/docs/tinymce/latest/plugins/), [Word export mechanism](https://www.tiny.cloud/docs/tinymce/latest/exportword/).
- BlockNote: [getting started](https://www.blocknotejs.org/docs/getting-started), [pricing and core licence](https://www.blocknotejs.org/pricing), [format interoperability](https://www.blocknotejs.org/docs/foundations/supported-formats).

Plain textarea/Markdown would reduce the editor surface but would reverse the current approachable rich-text direction. A custom canvas editor would add text-input, selection and accessibility engineering; it is not a low-effort substitute. This is a bounded comparison of credible approaches, not an exhaustive list of libraries.

## What would we actually have to build?

Using an open-source editor does not mean rebuilding its text engine. We reuse that engine and implement the FAU behavior around it. Nor must we reproduce every feature advertised by a commercial platform.

| Feature | Work beyond an editor core | Relative uncertainty |
| --- | --- | --- |
| Formatting toolbar, tables, images | Configure supported nodes; accessible controls; upload authorization; paste restrictions; mobile testing | Moderate; prototype the exact configuration |
| Autosave and exclusive locks | Acknowledgement, retry/idempotency, revision checks, stale-write rejection and visible loss of lease | Moderate; required for every candidate, including paid products |
| Session snapshots/history | Existing server model, encrypted storage and restore/view UI if scheduled; editor undo is not durable version history | Moderate; paid editor history does not automatically replace this contract |
| Anchored comments | Threads, permissions, anchor mapping through edits, deleted-text/orphan handling, resolution and accessible navigation | High; substantially more than adding a comment sidebar |
| DOCX import/export | Conversion pipeline, schema mapping, assets, lists/tables and fidelity fixtures; warnings for unsupported content | High to very high for fidelity; a basic converter is not Word round-trip parity |
| Pagination and tracked changes | Page layout or change-attribution model, undo interactions, import/export semantics | High; only evaluate if explicitly required |
| Controlled publication | Separate public snapshot, private-block/comment exclusion, authorized assets and safe rendering | Moderate-to-high; FAU-specific regardless of editor vendor |
| Simultaneous editing | Conflict handling, shared state, presence and reconciliation with audit/permissions | High; currently deferred, so exclude from baseline cost |

These are uncertainty categories, not person-week commitments. Giving delivery estimates without a representative prototype would imply evidence we do not have.

## Cost comparison and evidence needed

Compare total cost over an agreed period: licence/subscription + integration + custom features + hosting + upgrades/testing + eventual migration. The free-core options have zero core licence fee, not zero engineering cost. Paid candidates remain unpriced for FAU until we specify features, developer seats, usage, deployment and support; public entry prices are not comparable quotes for this feature set.

Before final selection, use identical synthetic documents to test formatting, nested lists, tables, image/attachment references, copy/paste, mobile input, keyboard/screen-reader behavior, save failure and lost lease. Test htmx navigation without discarding unsaved editor state. Measure the complete editor bundle, not vendor core-only figures.

For the shortlisted models, demonstrate Rust-side validation and safe rendering, schema versioning, encrypted save/restore and content export. For comments, exercise insertion/deletion across anchors and undo. For DOCX, agree tolerated losses and evaluate fixtures before claiming support. Record exact package versions/licences and maintainers' integration estimates. No benchmark, vendor quote or accessibility pass is claimed here.

## Recommendation

Shortlist **Tiptap open-source core and Lexical** for the same small vanilla-JavaScript integration trial inside Rust-rendered HTML/htmx. Keep **CKEditor 5 as the packaged commercial comparator** if comments or DOCX fidelity are needed early; compare its scoped offer with Tiptap's paid option before assuming either is cheaper than custom development.

My provisional first candidate is **Tiptap's open-source core**, because its extension layer and ProseMirror model fit the currently proposed structured document contract without requiring an application-wide framework. This is an integration-risk judgment, not a finding that it has the best accessibility, lowest total cost or cheapest advanced features. Prior preference alone is not a reason to select it. Lexical should win if the same trial shows clearer maintenance and acceptable model/rendering work.

Do not adopt Tiptap Platform by implication. If reliable anchored comments and high-fidelity Office conversion are immediate requirements, compare the paid route against explicit implementation estimates before selecting the core. If those features can wait, keep the MVP editor narrow and avoid paying for or recreating a whole platform. Human review on #3493 selects the shortlist and feature priorities; final package adoption follows the missing evidence.

## Current recommendation

Implement Lexical in the React editor page and retain HTML/HTMX elsewhere. Validate accessibility, editor lifecycle, autosave/leases, schema round trips, Rust rendering, bundle size and package licences through #3422/#3490.
