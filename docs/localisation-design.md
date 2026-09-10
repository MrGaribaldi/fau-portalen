# Localisation and translation design

Card: #3439. Condition on Erik's acceptance of ADR-001 (#3411, 9 September 2026): "We should add
support for nynorsk/oversettelser. Do not actually write ny nynorsk, that will be done by someone
else." With "Bokmål is standard" from the same thread.

Status: specification for review. No Nynorsk text is written here or anywhere in the repository,
by design. This document delivers the mechanism and the rules; the translation itself is a human
task.

## Scope, and the three kinds of text

Localisation questions collapse into confusion unless these are kept apart:

1. **Interface text** — labels, buttons, validation messages, email bodies. Written by us,
   translated by a translator. This is what "supporting Nynorsk" means.
2. **Document content** — meeting minutes, plans and notes written by FAU members. Never
   translated by the platform. An FAU writing in Nynorsk is simply an FAU whose documents are in
   Nynorsk.
3. **Reference data** — school and municipality names from the register (#3441). Not translated;
   these are proper nouns from an official source.

Only category 1 gets a translation pipeline. Categories 2 and 3 still need a *language tag*, for
accessibility and correct rendering, which is a smaller and different requirement.

## Locale model

- Locales are BCP 47 tags, not a boolean. `nb-NO` (Bokmål) is the default and the fallback;
  `nn-NO` (Nynorsk) is the first added translation. Nothing in the design may assume Norwegian
  only: FAU-er at schools with many immigrant families are a realistic later case for English or
  another language, and the mechanism should not have to be rebuilt for that.
- Two places store a preference, because they answer different questions:
  - `account.locale` (nullable) — what this person wants to read. Follows them across every FAU
    they belong to, since the account is global in the #3412 model.
  - `tenant.default_locale` (not null, defaults to `nb-NO`) — what this FAU uses for anyone
    without a preference, and for text sent on the FAU's behalf.
- Resolution order for a request, first match wins: explicit switch in this session → the
  account's locale → the active tenant's default → `Accept-Language` → `nb-NO`. Anonymous public
  pages skip the middle two.
- Both are additive nullable/defaulted columns on entities that already exist in the accepted
  #3412 model, so this is an amendment to ADR-001 and #3412 rather than a redesign.

## Interface text

- **Catalogue format:** ICU MessageFormat, one JSON file per locale, `frontend/src/locales/nb-NO.json`
  and `nn-NO.json`. ICU rather than plain key/value because plurals and gendered or inflected
  forms differ between Bokmål and Nynorsk, and string concatenation in code makes translation
  impossible. Keys are semantic (`document.lease.takenBy`), never English sentences.
- **No natural-language keys and no fallback to the key.** A missing translation falls back to
  `nb-NO`; if a Bokmål string is missing, that is a build failure, not a runtime surprise.
- **Bundling:** one hashed asset per locale under `/assets/`, loaded for the resolved locale
  only. This fits the ADR-001 contract as written — hashed assets cache long, HTML revalidates,
  and a missing asset returns 404 rather than an HTML fallback. Do not inline all locales into
  the main bundle; the cost grows with every language added.
- **Server-side strings:** the API must not return human-readable Norwegian as its payload.
  ADR-001 already specifies JSON errors with a stable error code and request ID — that contract
  is what makes localisation possible, and it must be honoured strictly: the server returns
  codes and parameters, the client renders the sentence. The only text the server renders itself
  is email.

## Email

- Templates live in the application, one per locale, rendered server-side. The provider
  transports; it does not own the templates. This preserves the provider-independence #3410
  requires and keeps a provider switch from becoming a translation project.
- The **outbox row carries the locale resolved at enqueue time** (`outbox.locale`). A message
  queued for a member who then changes their preference must not silently change language, and a
  retry must produce the same text. This follows the existing outbox design in #3412.
- Recipient locale: the account's locale if the recipient is a known member, otherwise the
  tenant default. For invitations to an address that has no account yet (#3417 invitation flow),
  use the inviting tenant's default and let the recipient switch after signup.
- Any locale-specific sender name or footer must not change the envelope sender or the signing
  domain, which stay as decided on #3410.

## Document content and history

- **Documents are not translated.** No machine translation, no translated copies. Storing a
  translated variant would fork the version history and the audit trail, which the #3412 design
  deliberately keeps single-threaded per document.
- Add an optional `document.language` (defaulting to the tenant default at creation) used to set
  the `lang` attribute when rendering. This is an accessibility requirement, not a nicety:
  screen readers switch pronunciation on it, and WCAG's language-of-parts criterion applies to
  the universal-design work on #3415.
- **Historical versions keep the language they were written in.** A finalized `document_change`
  snapshot is immutable, so it is never re-tagged or re-rendered in another language. If an FAU
  switches language, old versions stay as written — which is correct, and also the only
  behaviour consistent with an immutable audit trail.
- **Audit stays language-neutral by construction.** The #3412 model already stores `action` plus
  bounded metadata rather than rendered sentences, and this design depends on that: audit rows
  must never contain a pre-rendered Norwegian sentence, because history would then be frozen in
  the writer's locale. The client renders audit entries from the action code and its parameters,
  exactly as it renders errors.

  Correction to an earlier version of this document, after Erik's comment of 9 September 2026:
  the argument was overstated as Bokmål history being "unreadable" to a Nynorsk reader. It is
  not — Nynorsk readers read Bokmål without difficulty, and Erik confirms this is not a deal
  breaker. The rule stands on two narrower grounds: it is the only way a third language, such as
  one spoken by immigrant families at a school, ever becomes possible without rewriting history,
  and rendering from codes is simply the cleaner contract. Nothing here is urgent on Nynorsk's
  account.

## Formatting

Use the platform `Intl` APIs with the resolved locale for dates, times, numbers and lists; do
not hand-roll Norwegian formats or reuse a Bokmål format string for Nynorsk. Times display in
`Europe/Oslo`. The `school_year` naming from #3412 is FAU-authored data, not a formatted value,
so it is displayed as stored.

## How a translator works without repository access

Agents must not write Nynorsk, and the translator will not have a checkout. Proposed round trip:

1. A maintainer exports the Bokmål catalogue plus any existing target strings to one file
   (XLIFF, or CSV if the translator prefers a spreadsheet), including the key, the Bokmål source,
   a short context note per key, and a screen reference.
2. The translator fills in target strings and returns the file.
3. A maintainer imports it, which regenerates `nn-NO.json`, and CI checks that every key parses
   as valid ICU and that no key is missing from `nb-NO`.
4. Missing target keys are allowed and fall back to Bokmål, so a partial translation ships
   safely and can be completed incrementally.

This keeps the source of truth in the repository, needs no external translation service, and
therefore adds no supplier to #3409. A translation SaaS would be a customer-facing-adjacent
supplier decision and is deliberately avoided.

## What this amends in ADR-001

For the record, so the ADR can be updated in one pass:

- Locale resolution order and the two preference fields.
- Per-locale hashed asset bundles under `/assets/`, one loaded per request.
- Strict restatement that API errors carry codes and parameters, never display text.
- Email templates rendered in the application, with the locale captured on the outbox row.
- `<html lang>` set from the resolved locale, and `lang` set from `document.language` when
  rendering document content.
- Two additive columns (`account.locale`, `tenant.default_locale`) and one optional
  (`document.language`).

## Decisions taken, 9 September 2026

- **Stable error codes: confirmed.** Erik: "I like stable error codes, so we go for that." The
  API returns codes and parameters, never display text, and audit stores action codes. This is
  now a decision rather than a proposal, and it binds ADR-001 and #3412.
- **Nynorsk is design-only for now.** Erik: "Nynorsk is just for design right now, we stick with
  Bokmål for now since I want a proper translation when we launch that." So the deliverable is
  the mechanism plus a complete Bokmål catalogue. No `nn-NO.json` is created, not even a stub —
  an empty or machine-guessed catalogue is worse than none, because it would ship half-translated
  interface text under a language switch. The switcher stays hidden while only one locale exists.
- **Translation happens at launch, by a human translator.** The export/import round trip below
  is therefore built and tested with the Bokmål catalogue as both source and target, so the
  pipeline is proven before a translator is engaged, without producing any Nynorsk text.

## Decisions still needed

- **Nothing blocking.** The remaining questions are timing, not design:
- **URL strategy for public pages**, decidable when the second locale actually arrives. A locale
  path prefix (`/nn/`) is better for search visibility on the landing page (#3423); a cookie plus
  `Accept-Language` is simpler for the authenticated app (#3422). Recommended: path prefix for
  public marketing pages only, cookie inside the app. Nothing needs building now, but the
  routing in #3422 and #3423 should not make a path prefix expensive to add later.
- **Any third language to plan for**, so the pipeline is tested with more than one target before
  it is assumed to generalise.
- **Translator and format** — who does it, and whether they want XLIFF or a spreadsheet.

## Not verified, not done

No frontend framework is chosen yet (#3415), so the specific i18n library is left open on
purpose; the constraints above (ICU catalogues, per-locale bundles, no display text from the
API) are what the choice must satisfy. No Nynorsk text was written. No translation supplier was
contacted. Nothing was implemented.
