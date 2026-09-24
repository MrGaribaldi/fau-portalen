# School and municipality register: design for #3441

Status: **decided 24 September 2026.** Erik answered D1–D11 on #3441. Section 12 records each
answer, and docs/planning-decisions.md ("School register decided (#3441)") records the agent's
rulings on the parts his answers added. Those rulings are the Brreg FAU matching in 2.5 and 4.6,
the weekly cadence and the submission lookup in 5.4. The draft was written 23 September 2026
overnight, as authorised in docs/planning-decisions.md ("#3416, #3413 and #3485 accepted").
User-facing strings are Bokmål source strings for the localisation mechanism (#3439), shown in quotes.

Every online fact below was checked on **23 September 2026** unless another date is given. Figures
marked *(measured)* were computed on that date from a full download of the public NSR API into a
scratch directory, which has since been discarded. Anything that could not be verified is marked
**Unverified**.

## 1. Why this is on the critical path

The approved flow (docs/fau-creation-and-membership-flow.md §3.1, decision 1) has a registrant pick
the school from the national register, municipality first and then school, with "Mangler skolen
din?" as the fallback. The database enforces one live FAU per school (§3.2, §9). So #3417 cannot build
signup until the register exists, and the register is also the source of URL identity for the public
pages in ADR-002 (docs/url-scheme.md). This document fixes:

- the source;
- the scope, meaning which schools can have an FAU;
- the schema;
- import and sync;
- slug minting and history;
- the picker;
- what the public may see.

It does **not** authorise outreach. The outreach plan still needs Erik's authorisation and the
legal and privacy check on #3426 and #3427 (docs/planning-decisions.md, 9 September).

## 2. Research findings

### 2.1 The authoritative source: Nasjonalt skoleregister (NSR)

NSR is run by Utdanningsdirektoratet (Udir). Its API describes itself as "enheter fra
Brønnøysundregisteret (Brreg) som tilhører grunnutdanningen", reconciled with Brreg, plus Norwegian
schools abroad and "noe tilleggsinformasjon utover det som fins i Brreg" (API description in
`https://data-nsr.udir.no/swagger/v4/swagger.json`).

| Question | Finding |
|---|---|
| API | REST, JSON, at `https://data-nsr.udir.no`. **v4 is current** (spec version "4.0"). v3 still answers at `/v3/enheter`. |
| Authentication | **None for the public data.** Every `/v4/enheter…` and `/v4/enhet/{orgnr}` call worked anonymously. Only `/v4/innlogget/enhet/{orgnr}` requires basic auth, and it is the only endpoint that adds contact data (`Epost`, `Telefon`, `Mobil`). We do not need it and should not ask for it. |
| Bulk download | No file download was found. The paged list `GET /v4/enheter?sidenummer=N&antallperside=1000` is effectively a bulk export: 19 pages of 1,000 covered all 18,349 units *(measured)*. The Felles datakatalog entry also lists a separate "NXR fellestjeneste" API (`https://data-nxr-fellestjeneste.udir.no`), which is not needed. |
| Incremental | `GET /v4/enheter/endretetter?dato=YYYY-MM-DD` returns units changed after a date, in Norwegian time. It is coarse: it returned 6,147 units for "after 1 June 2026" *(measured)*, which is probably a bulk re-stamp such as the NACE 2025 code update. Treat it as a hint, not a precise change feed. |
| Other endpoints | By municipality, county, school category and NACE code, plus the code lists `/v4/skolekategorier`, `/v4/utgaattyper`, `/v4/relasjonstyper`, `/v4/maalformer` and `/v4/organisasjonsformer`. |
| Licence | **NLOD** ("Norsk lisens for offentlige data"), stated in the API spec and on data.norge.no. NLOD 2.0 §5 requires attribution. Where the licensor specifies none, the default wording is: «Inneholder data under Norsk lisens for offentlige data (NLOD) tilgjengeliggjort av Utdanningsdirektoratet». It also requires changes to be marked clearly, which matters for curated display names (decision D5). |
| Update frequency | "Data importeres fra Brreg i tidsrommet 01:00-02:00", so nightly. The data.norge.no entry says daily. |
| Change notices | Udir invites API users to email `nxr-teknisk@udir.no` to receive notices of changes and outages. |
| Rate limits | **Unverified.** None are documented, and no rate-limit headers were returned. 2,733 detail calls at four in parallel took 51 seconds with no errors. |
| Supplier rule | Udir is a Norwegian state agency, and the data is public open data about institutions. It satisfies the customer-facing European rule, and no DPA question arises because no personal data flows either way. |

**Fields in the full record** (`NsrEnhetApiModel`). Those the design uses are in bold:

- **`Organisasjonsnummer`**;
- **`Navn`**, `FulltNavn`, `Karakteristikk`;
- **`Kommune`** {`Navn`, **`Kommunenummer`**, `Organisasjonsnummer`, `ErNedlagt`, `Fylkesnummer`} and **`Fylke`** {`Fylkesnummer`, `Navn`};
- **`Beliggenhetsadresse`** and `Postadresse`, each {`Adresse`, `Postnummer`, `Poststed`, `Land`};
- `Koordinat`;
- **`Internettadresse`**;
- **`Maalform`** (B/N);
- `Organisasjonsform`;
- **`Naeringskoder`**;
- **`Utgaattype`** and **`UtgaattDato`**;
- the flags **`ErAktiv`**, `ErEkskludert`, **`ErSkole`**, **`ErGrunnskole`**, **`ErPrivatskole`**, **`ErOffentligSkole`**, **`ErVideregaaendeSkole`**, `ErSpesialskole` and `ErGrunnopplaering`;
- `PrivatskoleGodkjenning` {`Godkjenningslov`, `GodkjentStatus`, …};
- `Elevtall` and `AntallAnsatte`;
- **`SkoletrinnGSFra`/`SkoletrinnGSTil`** and `SkoletrinnVGSFra`/`SkoletrinnVGSTil`;
- **`Skolekategorier`**;
- `ForeldreRelasjoner` and `BarnRelasjoner`;
- `OppstartsEllerStiftelsesdato`;
- **`DatoEndret`**.

The paged list returns a slimmer model with the flags, the category list, `Kommunenummer` and
`DatoEndret`.

**Identifiers and how they behave** *(measured)*:

- The stable external ID is the **organisation number of the school's sub-unit** (Brreg
  "underenhet", `Organisasjonsform` BEDR for all 2,733 active grunnskoler). It survives county
  renumbering. Hosle skole keeps `974552124` through Bærum's change from 3024 to 3201, and the Haram
  schools kept their 1969-era numbers through the 2024 split from Ålesund.
- It does **not** always survive a reorganisation. Stange ungdomsskole was `975270920` from 1969
  until it was marked "Slettet for sammenslåing" in August 2024. It reappears as `933181995`, same
  name, same municipality, started 1 August 2024. Røros skole has two numbers too. So the organisation
  number is an attribute that can change under a continuing school, exactly as ADR-002 decision 1
  assumed, and not our key.
- Organisation numbers are **not always numeric**: 21 Norwegian schools abroad use pseudo-numbers
  such as `U90099013` under pseudo-municipality 2599. Store them as text.
- **Closures carry a reason code but no successor.** `/v4/utgaattyper` gives:
  - A: none;
  - D: "Slettet for dublett";
  - F: "Slettet for sammenslåing";
  - N: a change of industry code;
  - O: the owner is a sole proprietorship;
  - S: "Slettet";
  - U: the owner has no active business.

  `UtgaattDato` dates the closure. None of the merged schools inspected had a relation pointing at
  its successor, which means **mergers cannot be followed automatically**: a human links the
  successor.
- Closed units keep the municipality number they had when they closed. Examples are Lysaker skole
  under 1719, Levanger's pre-2018 number, and Hagen skole under 3031, Nittedal before 2024. Inactive
  rows must never be used to build municipalities.

**Volume on 23 September 2026** *(measured)*:

| Set | Count |
|---|---|
| All NSR units, including owners and closed | 18,349 |
| Units with `ErSkole` | 10,515 |
| Active schools | 5,685 |
| Active with `ErGrunnskole` | 2,733, of which 2,443 public and 290 private |
| …of which adult education (category 10 or 25, or NACE 85.593) | 31 |
| …of which primary NACE code is upper secondary (85.3xx; hospital schools such as Viti skole) | about 17 |
| …of which abroad (pseudo-municipality 2599) | 8 |
| **Candidate set after the section 2.3 filter** | **2,677**, of which 274 private, across 357 municipalities plus Svalbard |
| Combined schools with both grunnskole and VGS grades | 50 |
| No pupil count (`Elevtall` null or 0) in the candidate set | 103. Many are newly started schools. |
| No website | 550 of 2,733 |
| Målform Nynorsk | 439 of 2,733 |
| Most schools in one municipality | Oslo 158, Bergen 93 |
| Two active schools with the same name in the same municipality | **0** |
| Slug collisions within a municipality, using ADR-002's transliteration | **0**. The longest slug is 72 characters. |

**Name quality is uneven.** NSR `Navn` is the Brreg name. Some candidates carry administrative
noise:

- 41 start with "<Kommune> kommune", for example "Holtålen kommune Hov skole" and "Elverum kommune -
  Ydalir skole";
- 128 contain "avd"/"avdeling", for example "Venn oppvekstsenter avd skole";
- a few are not schools a parent would recognise: "Porsgrunn kommune Vikarer Grunnskole/Sfo",
  "Klepp kommune Klepp Tverrfagelege Team", "Tysvær Morsmålsopplæring".

This drives decisions D2 and D5.

### 2.2 Municipalities: Kartverket and SSB

NSR embeds municipality data, but its `Kommune.Navn` is the full official multilingual name, for
example "Aarborte - Hattfjelldal", "Trondheim - Tråante" and "Nordreisa - Ráisa - Raisi". It also
carries historical numbers on closed rows. It is not a municipality register. Two official sources
are:

| Source | What it gives | Licence | Notes |
|---|---|---|---|
| **Kartverket, Administrative enheter API** `https://api.kartverket.no/kommuneinfo/v1` (the old `ws.geonorge.no/kommuneinfo/v1` proxies to it) | 357 current municipalities; `kommunenummer`; `kommunenavn`, which is the priority-1 official name; `kommunenavnNorsk`; `gyldigeNavn`, every official name with language and priority; county number and name; `samiskForvaltningsomrade` | **CC BY 4.0** (Geonorge metadata; maintenance "Årlig", updated 30 July 2026) | No authentication. Next year's data is published on `api.test.kartverket.no` in December (it was for 2024). Incompatible changes are announced three months ahead on status.kartverket.no. |
| **SSB Klass, classification 131** "Standard for kommuneinndeling" `https://data.ssb.no/api/klass/v1/classifications/131` | Codes valid at a date (`/codesAt`), and **changes between two dates** (`/changes?from=…&to=…`) as `oldCode → newCode` pairs | **CC BY 4.0** (SSB's APIs, per ssb.no/api) | 358 codes including 9999 "Uoppgitt". Names are the full official form. |

What the change feed shows *(measured)*:

- **1 January 2024: 118 code changes.** County reversals renumbered whole counties: 3024 Bærum became
  3201, 3811 Færder became 3911, 5401 Tromsø became 5501. There was one split, 1507 Ålesund into 1508
  Ålesund and 1580 Haram. Several name-only changes also happened, such as 1833 "Rana" becoming "Rana
  - Raane".
- **2024 to 2026: 6 changes, all on 1 January 2026.**
  - "Oslo" became "Oslo - Oslove", "Steinkjer" became "Steinkjer - Stïentje", and "Lyngen" became
    "Lyngen - Ivgu - Yykeä".
  - 3118 Indre Østfold is listed as changing to 3207 Nordre Follo and 3216 Vestby while 3118
    continues. That is a **boundary adjustment** rather than a merger, and a naive sync would read it
    as one.
- Kartverket gives Oslo's `kommunenavn` as "Oslo" while SSB says "Oslo - Oslove". The sources
  differ on the multilingual form. Both agree on the Norwegian name.

Twelve municipalities have a Sámi or Kven priority-1 name in Kartverket, for example Gáivuotna
(Kåfjord), Kárášjohka (Karasjok) and Guovdageaidnu (Kautokeino). Their names contain á, š, č, đ,
ŋ, ŧ, ž and ï.

**Svalbard is not a municipality** in either source. NSR files Longyearbyen skole under 2100.
**Unverified:** that opplæringslova's FAU rule applies on Svalbard. The Svalbard regulation on
education was not checked.

### 2.3 Which schools have an FAU

- **Public grunnskole:** opplæringslova (LOV-2023-06-09-30, in force 1 August 2024) § 10-5: "Kvar
  grunnskole skal ha eit arbeidsutval som er valt av foreldra på skolen. Foreldra kan velje å
  organisere seg på andre måtar." Mandatory, with an escape clause.
- **Private grunnskole:** privatskolelova (LOV-2003-07-04-84; its current short title is
  *privatskolelova*, not friskolelova) § 5A-5: "Kvar grunnskole skal ha eit foreldreråd … Foreldrerådet
  **kan** velje eit arbeidsutval." Optional, but common enough to include. ADR-002 §8 called the
  approval route "friskolelova". That was corrected on 24 September 2026, and the reviewer checklist
  should say privatskolelova too. NSR records approval under "privatskoleloven" for 285 schools and "opplæringsloven
  § 22-1" for 5.
- **Videregående is out of scope.** The same § 10-5 requires only an elevråd at upper secondary and no
  parents' body. The product premise is FAU.
- **Combined schools**, meaning grunnskole plus VGS (50), are **in**, because they have a grunnskole
  part.
- **Special schools** at grunnskole level (NACE 85.202; 18 flagged `ErSpesialskole`) are
  grunnskoler and are **in**.
- **Out:**
  - adult education on grunnskole level, which has no parents' body;
  - units whose primary NACE code is upper secondary;
  - Norwegian schools abroad, because opplæringslova does not govern them and pseudo-municipality 2599
    has no place in `/fau/<kommunenr>-<navn>`.

This gives the filter used for the 2,677 candidate set:

```
ErSkole AND ErAktiv AND ErGrunnskole
AND no category 10 (Voksenopplæringssenter) or 25 (Voksenopplæring på grunnskolens område)
AND primary NACE code not 85.593 (adult education) and not 85.3xx (upper secondary)
AND Kommunenummer <> '2599'
```

The filter keeps the few oddities in 2.1, such as a substitute pool and an interdisciplinary team.
They are harmless in a picker nobody searches for them in, and an operator can mark them out of scope
(D2).

### 2.4 Renames and renumbering against ADR-002

ADR-002 already makes the pair `<kommunenr>-<navn>` the municipality segment, because number and name
change independently. The data above adds three cases the sync must handle:

1. **Renumber, same name** (3024 Bærum becoming 3201 Bærum). Same entity, new number, new slug
   `3201-baerum`. The old slug stays in history and 301s (ADR-002 resolution rule 2). Every school
   path under it changes too, which the school-slug history handles (section 4.2).
2. **An official name change that does not change the Norwegian name** ("Oslo" becoming "Oslo -
   Oslove"). If slugs followed the official name, every Oslo URL would 301 on 1 January 2026. They
   should follow the Norwegian name (D3), so nothing changes.
3. **Split, merger and boundary adjustment.**
   - A one-to-one SSB change is an automatic renumber.
   - One-to-many or many-to-one changes go to an operator, who decides which entity keeps the UUID.
     For 1507 Ålesund, the entity continues as 1508 Ålesund and Haram gets a new UUID.
   - A change whose old code still exists afterwards is a boundary adjustment and is ignored.
   - Schools that move municipality in a split keep their UUID. Their old paths 301 through history.

### 2.5 The FAU-er themselves: Brreg's Enhetsregisteret (D1)

Most FAU-er are registered in Brreg as their own entities, usually organisation form FLI
("Forening/lag/innretning") with NACE 94.992. Erik's D1 answer links them to their schools, so an
FAU whose name differs from the school's is named correctly from the start. Measured on
24 September 2026:

| Question | Finding |
|---|---|
| Source | `https://data.brreg.no/enhetsregisteret/api/enheter/lastned`, a gzip JSON array of every registered main unit: 1,175,297 units, 210 MB, about 40 s to download. Anonymous. Refreshed nightly. Licence NLOD, like every Brreg open dataset. |
| Search API | Not usable for this. `navn=fau` returns 3,732 hits, including names such as "Fauske …". `navn=arbeidsutvalg` returns fewer hits than `navn=foreldrenes arbeidsutvalg`, so the filter is not a substring match. FLI with NACE 94.992 is 73,355 rows, and the API cannot page past 10,000. The CSV bulk endpoint needs authentication. |
| Candidate set | FLI units whose name contains FAU, "foreldrenes arbeidsutvalg", "foreldrerådets arbeidsutvalg", "arbeidsutval(g)" or "foreldreråd(et)" on word boundaries: **2,477**. All but 14 have NACE 94.992. Some are kindergarten parents' councils, and those simply fail to match a school. |
| Personal data | **913 of 2,477 carry a `c/o` or `v/` address line**, typically a named parent at a home address, e.g. "c/o <name>, <home street>". Brreg names are institutional ("BESTUM FAU"). |
| Address match | Street line plus postcode, normalised, against the school's NSR visiting and postal addresses, ignoring `c/o`, `v/` and post-box lines: **1,256** FAU-er match exactly one school, **29** match several, and 1,192 match none. 28 schools have more than one FAU at their address. |
| Name check | Where both address and a name core (the FAU name without FAU words, compared with the school name in the same municipality) identify a single school, they agree **876** times and disagree **4** times. The name alone identifies a single school for another **360** FAU-er with no address match. |

So the address is a strong signal and the name a useful second one. The rules built on this are in
4.6.

## 3. Principles

1. **Identity is ours.** Every municipality and school row has a UUIDv7 we assign. Organisation
   numbers, municipality numbers and names are versioned attributes (ADR-002 decision 1).
2. **The register is global, public reference data**, not tenant data (flow §9). It carries no
   `tenant_id` and is not encrypted: ADR-003's per-FAU encryption covers FAU content, and this is
   Udir's open data. The one exception is who submitted a school, which is personal data and lives in
   a separate table (4.4).
3. **Never hard-delete a school or municipality that anything references.** Foreign keys are
   `on delete restrict`. Closure is a state.
4. **Slugs are stored, never derived at render time**, and never edited in place. A change closes the
   old slug into history and issues a new one (Erik's comment on #3441, 9 September).
5. **Automatic where the data is unambiguous, human where an FAU is affected.** The sync may rename,
   renumber and close unattached rows by itself. Anything touching a school that has an FAU, and every
   merger or split, becomes a review item for Erik.
6. **The register holds nothing about individuals.** The product never calls NSR's authenticated
   contact endpoint. Outreach contact data stays in #3431's prospect register, outside the product
   database. Brreg FAU addresses are used only in memory, during matching (4.6). They are never
   stored, logged, put in a review item or shown, because many of them are a parent's home
   address.

## 4. Data model

This belongs in its own migration, numbered after the 0003 membership migration that is being built
without the register (docs/planning-decisions.md, 23 September). That is "0004" below, subject to
merge order. The SQL is a **shape sketch** for review, not final DDL. It follows 0002's conventions:

- UUID primary keys;
- `timestamptz`;
- `text` with `check` constraints rather than enums;
- explicit grants;
- half-open validity periods;
- no column name containing `key`, `secret`, `cipher` and so on, since `schema_review.rs` fails on a
  substring (it rules out names like `source_key` and `idempotency_key`).

### 4.1 Municipalities

```sql
create table municipalities (
  id              uuid        primary key,              -- UUIDv7, ours
  name            text        not null,                 -- Norwegian name (kommunenavnNorsk): "Kåfjord"
  official_name   text,                                 -- full official form: "Gáivuotna - Kåfjord - Kaivuono"
  county_number   text        not null check (county_number ~ '^[0-9]{2}$'),
  county_name     text        not null,
  slug            text        not null,                 -- current segment, e.g. '5540-kaafjord'
  status          text        not null check (status in ('active', 'dissolved')),
  dissolved_on    date,
  source          text        not null check (source in ('kartverket', 'manual')),  -- Svalbard is manual
  source_checked_at timestamptz,
  search_text     text        not null,                 -- section 7; app-computed
  created_at      timestamptz not null default now(),
  updated_at      timestamptz not null default now()
);
create unique index municipalities_slug_current on municipalities (slug);

-- Every official name, in every language, for search: Gáivuotna, Kåfjord, Kaivuono.
create table municipality_names (
  municipality_id uuid    not null references municipalities (id) on delete restrict,
  name            text    not null,
  language        text    not null,                    -- BCP 47: 'no', 'se', 'fkv', 'sma', 'smj'
  priority        integer not null,
  primary key (municipality_id, name)
);

-- kommunenummer is an attribute with a validity period. NSR rows and SSB changes resolve
-- through here, including closed schools that still carry 1719 or 3031.
create table municipality_numbers (
  municipality_id uuid not null references municipalities (id) on delete restrict,
  number          text not null check (number ~ '^[0-9]{4}$'),
  valid_from      date not null,
  valid_until     date,                                -- exclusive; null = current
  primary key (number, valid_from),
  check (valid_until is null or valid_from < valid_until)
);
create unique index municipality_numbers_current on municipality_numbers (number) where valid_until is null;

create table municipality_slug_history (
  slug            text        not null,
  municipality_id uuid        not null references municipalities (id) on delete restrict,
  valid_from      timestamptz not null,
  valid_until     timestamptz not null,                -- closed entries only; current is on the row
  primary key (slug, valid_from)
);
```

A number is reused over time: 0716 was Våle, then Re. So `municipality_numbers` is keyed on
`(number, valid_from)`, and a lookup by number must say *when*. The nightly NSR sync only ever
resolves **current** numbers, because it imports active rows only.

### 4.2 Schools

```sql
create table schools (
  id                uuid        primary key,            -- UUIDv7, ours; /s/<id>, /bli-med/<id>
  municipality_id   uuid        not null references municipalities (id) on delete restrict,
  origin            text        not null check (origin in ('register', 'submitted')),
  -- Names. display_name is what we render and slug from; register_name is NSR's Navn as last seen.
  display_name      text        not null,
  register_name     text,
  display_name_curated boolean  not null default false, -- D5: set when an operator overrides
  slug              text,                               -- null until verified (ADR-002 §8)
  -- Verification: 'listed' = from NSR; 'pending' = submitted, awaiting review;
  -- 'verified' = submitted and accepted by the reviewer; 'rejected'; 'held' = NSR row held
  -- back while a possible match to a submitted school is reviewed (4.5).
  verification      text        not null check (verification in ('listed', 'pending', 'verified', 'rejected', 'held')),
  verified_at       timestamptz,
  -- Register attributes, all nullable for submitted schools.
  orgnr             text        unique check (orgnr ~ '^[0-9A-Z]{9}$'),
  ownership         text        check (ownership in ('public', 'private')),
  grade_from        smallint    check (grade_from between 1 and 13),
  grade_to          smallint    check (grade_to between 1 and 13),
  register_language text,                               -- BCP 47 from Maalform: 'nb' or 'nn'
  website           text,
  street_address    text,
  postcode          text,
  post_town         text,
  -- Scope and lifecycle.
  in_scope          boolean     not null default true,
  scope_override    boolean,                            -- operator decision; wins over the filter
  status            text        not null check (status in ('active', 'closed')),
  closed_on         date,
  closure_reason    text        check (closure_reason in ('closed', 'merged', 'duplicate', 'rejected', 'out_of_scope')),
  successor_id      uuid        references schools (id) on delete restrict,
  -- Provenance.
  source_changed_at timestamptz,                        -- NSR DatoEndret
  last_seen_in_source_at timestamptz,
  search_text       text        not null,
  created_at        timestamptz not null default now(),
  updated_at        timestamptz not null default now(),
  check ((status = 'closed') = (closed_on is not null)),
  check (origin = 'submitted' or orgnr is not null),
  check (slug is null or verification in ('listed', 'verified'))
);
-- ADR-002 §4: school slugs unique within a municipality, among current slugs.
create unique index schools_slug_current on schools (municipality_id, slug) where slug is not null;

-- Organisation numbers are versioned too (Stange ungdomsskole, 4.5).
create table school_orgnr_history (
  orgnr       text        primary key,                 -- an orgnr belongs to one school, ever
  school_id   uuid        not null references schools (id) on delete restrict,
  valid_from  timestamptz not null,
  valid_until timestamptz                               -- null = current
);

-- Keyed on the municipality *at the time*, so /fau/1507-aalesund/brattvaag-barneskule still
-- resolves after the school moved to 1580 Haram.
create table school_slug_history (
  municipality_id uuid        not null references municipalities (id) on delete restrict,
  slug            text        not null,
  school_id       uuid        not null references schools (id) on delete restrict,
  valid_from      timestamptz not null,
  valid_until     timestamptz not null,
  primary key (municipality_id, slug, valid_from)
);
```

**The foreign key from tenants**, in the same migration:

```sql
alter table tenants
  add constraint tenants_school_fk foreign key (school_id) references schools (id) on delete restrict;
```

The one-FAU-per-school partial unique index already exists: migration 0003 (#3418) created
`tenants_one_live_per_school` on `tenants (school_id)` where status is pending or active. The register
migration adds only the foreign key and must not create the index again. (Corrected 24 September
2026.)

This **supersedes the comment in 0002** on `tenants.school_id`. That comment predicted a
tenant-scoped `schools (tenant_id, id)` and a composite, deferrable foreign key. Flow §9 made schools
global, so a plain foreign key is correct and there is no circularity. The migration should say so
in its own header, since 0002 cannot be edited. Recommended in the same migration: make
`tenants.school_id` `not null`. Every FAU has a school, no tenants exist in production yet, and the
"Mangler skolen din?" path creates its school row in the same transaction.

The index covers a race too. Two concurrent signups for the same school both try to insert a
`pending` tenant, and the second fails on the index. #3417 maps that unique violation to the §3.2
message, never to a 500.

### 4.3 NSR staging and sync bookkeeping

```sql
-- Last NSR payload seen per organisation number, in scope or not. Provenance and debugging.
-- NSR only: Brreg payloads are never stored, because their addresses are personal data (4.6);
-- about 18,000 small rows if everything is kept, or about 2,800 if only grunnskoler are.
create table register_source_records (
  source          text        not null check (source in ('nsr')),
  external_id     text        not null,                -- orgnr
  payload         jsonb       not null,
  payload_sha256  text        not null,
  source_changed_at timestamptz,
  fetched_at      timestamptz not null,
  in_scope        boolean     not null,
  scope_reason    text        not null,                -- e.g. 'adult_education', 'abroad', 'upper_secondary'
  primary key (source, external_id)
);

create table register_sync_runs (
  id           uuid        primary key,
  kind         text        not null check (kind in ('seed', 'sync', 'dry_run')),
  started_at   timestamptz not null,
  finished_at  timestamptz,
  outcome      text        check (outcome in ('applied', 'no_change', 'aborted', 'failed')),
  counts       jsonb       not null default '{}',      -- created, renamed, closed, held, …
  abort_reason text
);

create table register_review_items (
  id          uuid        primary key,
  kind        text        not null check (kind in (
                'closure_with_fau', 'possible_reregistration', 'possible_submission_match',
                'municipality_split_or_merge', 'mass_change', 'unknown_municipality_number',
                'fau_several_at_school', 'fau_several_schools', 'fau_match_conflict',
                'submission_matches_listed_school')),
  school_id   uuid        references schools (id) on delete restrict,
  other_school_id uuid    references schools (id) on delete restrict,
  municipality_id uuid    references municipalities (id) on delete restrict,
  details     jsonb       not null,
  created_at  timestamptz not null default now(),
  resolved_at timestamptz,
  resolution  text
);
```

### 4.4 Submitted schools ("Mangler skolen din?")

ADR-002 §8 already specifies what a submission stores. The design splits it into two parts. The
school row holds only public facts. The submission row holds who asked, which is personal data whose
retention #3426 owns.

```sql
create table school_submissions (
  id               uuid        primary key,
  school_id        uuid        not null unique references schools (id) on delete restrict,
  submitted_by     uuid        not null references accounts (id),
  submitted_name   text        not null,                -- as typed; untrusted, escaped everywhere
  decision_url     text        not null,
  decision_kind    text        not null check (decision_kind in ('municipal_decision', 'udir_private_school_approval')),
  document_title   text,
  retrieval_status text        not null check (retrieval_status in ('fetched', 'not_allowlisted', 'failed', 'quarantined', 'attached_by_reviewer')),
  retrieved_at     timestamptz,
  -- Archived original and sanitised copy: object references per #3435 and ADR-002's ingest
  -- rules, stored in #3435's own tables, not here.
  review_state     text        not null check (review_state in ('pending', 'verified', 'matched', 'rejected')),
  reviewed_at      timestamptz,
  created_at       timestamptz not null default now()
);
```

A submitted school is created with `origin = 'submitted'`, `verification = 'pending'` and no slug.
It works fully at `/s/<uuid>` and is `noindex` (ADR-002; flow §3.1). By default it is **not** listed
in the picker or on the municipality page (D7).

**Row-level security.** `schools`, `school_submissions` and `register_lookups` carry RLS. `fau_app`,
the runtime role, may only ever *insert* a fresh, untouched-by-the-register row on each: a pending
submitted school with no slug, orgnr, curated-name flag, closure, successor, scope override,
provenance timestamp or register name; a submission still `pending` and unreviewed, with a
`retrieval_status` the fetcher itself could have produced (never `attached_by_reviewer`); a lookup
still unqueued and unprocessed. It has no update or delete grant on any of the three, and no select
grant on `school_submissions` or `register_lookups` at all. `fau_register` alone reads, updates and
(for a `held` school only) deletes. A future role granted none of this sees zero rows on these
tables by default -- RLS with no matching policy is deny-by-default, not merely un-granted.

### 4.5 How a submitted school meets NSR later, and how re-registrations are handled

**A submitted school appears in NSR.** When the sync sees a new in-scope organisation number:

1. It computes trigram similarity against pending or verified submitted schools **in the same
   municipality**.
2. Above a threshold (0.45 to start; tune it on real review outcomes), it creates the NSR school
   with `verification = 'held'` and raises `possible_submission_match`.
   - A held row is not pickable, so nobody can open a second FAU on the NSR copy in the meantime.
3. Erik then resolves the item one of two ways.
   - **Match.** The submitted row absorbs the register data: orgnr, attributes, `register_name`,
     and a slug if it lacks one. Its `verification` becomes `verified`, and the held NSR row is
     removed. Nothing references a held row, so deleting it is safe. The FAU, the tenant and
     `/s/<uuid>` are untouched, because the UUID never changes. The submission becomes `matched`.
   - **No match.** The held row becomes `listed`.

**A continuing school gets a new organisation number** (the Stange ungdomsskole pattern). When one
number is closed as "Slettet for sammenslåing" or "Slettet", and a new number appears in the same
municipality with the same or a very similar name within 30 days:

- **If no FAU references the old row**, the sync applies it automatically: it closes the old row,
  creates the new one and hands over the slug (ADR-002 rule 3, "the current holder wins"). The old row
  stays reachable at `/s/<old>`.
- **If an FAU references it**, the sync raises `possible_reregistration` and holds the new row. Erik
  chooses one of three outcomes:
  - **Same school.** The new orgnr is attached to the existing UUID, and the old one closes in
    `school_orgnr_history`. The FAU notices nothing.
  - **Successor.** A new UUID is created, the old row closes with `closure_reason = 'merged'` and
    `successor_id`, and `/s/<old>` 301s to the successor per ADR-002's error table.
  - **Unrelated.**

**A genuine merger of two schools that both have an FAU** is left to the review, which records the
successor on every closed school. `successor_id` is many-to-one. **Merging FAU-er is a product
requirement** (Erik, 24 September): a new FAU is created on the successor school, the old FAU-er's
documents are shared into it read-only, and their members keep read-only access to the old FAU-er.
The one-live-FAU index allows exactly that, because the old tenants stay on closed schools. The
sharing itself depends on the document layer (#3419) and per-document keys (ADR-003 decision 5),
and is its own card, #3498. Until it exists, the review records the successor and the FAU-er carry on
where they are.

**A school with an FAU closes.** The FAU keeps working, because closure changes nothing in the
workspace. The public page returns 410 with a pointer to the municipality page (ADR-002), and the
sync raises `closure_with_fau`. Nothing is deleted.

### 4.6 Brreg FAU entities and their links (D1)

```sql
-- One row per Brreg entity that looks like an FAU (2.5). No address column, on purpose.
create table registered_faus (
  orgnr               text        primary key check (orgnr ~ '^[0-9]{9}$'),
  registered_name     text        not null,            -- Brreg navn, as registered (capitals)
  organisation_form   text        not null,            -- 'FLI'
  municipality_number text,                            -- forretningsadresse.kommunenummer
  status              text        not null check (status in ('active', 'deleted')),
  last_seen_in_source_at timestamptz not null,
  created_at          timestamptz not null default now(),
  updated_at          timestamptz not null default now()
);

create table school_fau_links (
  school_id   uuid        not null references schools (id) on delete restrict,
  fau_orgnr   text        not null references registered_faus (orgnr) on delete restrict,
  method      text        not null check (method in ('address', 'name', 'operator')),
  -- linked: used for signup prefill; candidate: a name-only match awaiting confirmation;
  -- rejected: an operator said no, and the sync never proposes the pair again.
  state       text        not null check (state in ('linked', 'candidate', 'rejected')),
  created_at  timestamptz not null default now(),
  updated_at  timestamptz not null default now(),
  primary key (school_id, fau_orgnr)
);
create unique index school_fau_links_one_per_school on school_fau_links (school_id) where state = 'linked';
create unique index school_fau_links_one_per_fau    on school_fau_links (fau_orgnr) where state = 'linked';
```

**Matching, per weekly run**, entirely in memory:

1. For each candidate entity, compute its address keys: every address line with a house number,
   normalised and paired with its postcode. `c/o`, `v/` and post-box lines are skipped, with one
   exception (Erik, 24 September). A `c/o` or `v/` line that names the school, such as
   "c/o Hosle skole, Bispeveien 73", counts for that school only. It is checked against each
   candidate school sharing the postcode. A line naming a person or another school never counts.
2. Compute the same for every active in-scope school's visiting and postal address.
3. **One FAU, one school** at that address. The pair is linked with `method = 'address'`, unless
   the name core identifies a different single school. That raises `fau_match_conflict`, and
   nothing is linked.
4. **Several FAU-er at one school's address** raises `fau_several_at_school` (Erik's rule), and
   nothing is linked. **One FAU address matching several schools** raises `fau_several_schools`,
   also unlinked, even when the name would break the tie: a shared building is exactly where a
   guess goes wrong.
5. **No address match, and the name core identifies a single school in the FAU's municipality**,
   stores a `candidate` link. It is not used for prefill and not queued. It waits for the admin
   screen (D11b, #3499).
6. An operator link (`method = 'operator'`) or rejection is never overwritten by the sync. An
   entity missing from the bulk file becomes `deleted`, and its link stops being used.

**What a link is used for.** The signup form's FAU name defaults to the linked entity's registered
name, case-normalised (Brreg stores capitals, so "BESTUM FAU" becomes "Bestum FAU", keeping known
acronyms such as FAU and SFO). Otherwise it defaults to the school's `display_name`. It is a
suggestion like every other prefill, and it grants nothing.

## 5. Import and sync

### 5.1 One command, run two ways

`fau register sync` is a subcommand of the one binary (ADR-001: one image, one digest). It takes
`--dry-run`, which prints the diff and writes nothing but a `dry_run` run row, and `--seed`, which
permits creating everything on an empty register.

- **Seed.** Run once by an operator (Erik, or the agent with his go-ahead) against each environment
  after the migration, `--dry-run` first. The seed is simply the first sync.
- **Periodic sync.** A Kubernetes **CronJob** with the same image runs **weekly (D9), Mondays at
  04:30 Europe/Oslo**, after NSR's 01:00-02:00 Brreg import. It uses `timeZone: Europe/Oslo`, supported since Kubernetes
  1.27 (the cluster runs k3s v1.35.2), `concurrencyPolicy: Forbid`, and a PostgreSQL advisory lock
  inside the command, so a manual run and the CronJob cannot overlap. The manifest belongs to #3424's
  deployment work. Egress is limited to `data-nsr.udir.no`, `api.kartverket.no`, `data.ssb.no` and `data.brreg.no`.
- **Submission lookups** (5.4). `fau register lookups` runs every few minutes as its own CronJob,
  and exits at once when the queue is empty.

### 5.2 Order of work inside one run

1. **Municipalities first.** Read Kartverket's current list and SSB's changes since the last run:
   - a new code whose one-to-one SSB change points from a known code is a renumber (automatic);
   - a changed Norwegian name is a rename (new slug, old slug to history; automatic);
   - a change whose old code still exists afterwards is a boundary adjustment (ignored);
   - anything else is `municipality_split_or_merge`, and the run stops before touching schools
     in the affected municipalities.
2. **NSR list.** Page `/v4/enheter` fully. There are only 19 pages, so the full list is cheaper
   and more trustworthy than `endretetter`.
3. **NSR detail** for every grunnskole. The run is weekly, so it is always a full detail pass,
   which takes about a minute at four in parallel.
4. **Classify** each record in or out of scope (section 2.3, then `scope_override`), and upsert
   `register_source_records` by `(source, external_id)` with the payload hash.
5. **Apply to `schools`:**
   - new in-scope orgnr: create the school, or hold it (4.5);
   - `register_name` changed: set it, and unless the display name is curated, set `display_name`
     and re-mint the slug (section 6);
   - attribute changes: update;
   - became inactive or went out of scope: close. An attached FAU raises `closure_with_fau`
     instead of any silent change.
   - An NSR municipality number unknown to `municipality_numbers` raises
     `unknown_municipality_number` and skips that row. That happens on 1 January if NSR moves before
     Kartverket.
6. **Brreg FAU entities** (4.6): stream the bulk file, keep the candidate set, match and link.
7. **Record** counts in `register_sync_runs`, and write one `audit_events` row per applied run. The
   table comes forward in 0003 (flow decision 14). Register changes are system events with no tenant,
   so this needs #3421 to accept a null `tenant_id` for system events, or a separate system audit
   table. That is flagged for #3421, not decided here.

### 5.3 Idempotency and safety

- Keys are natural: `(source, external_id)` for staging, `orgnr` for schools, and the Kartverket
  number for municipalities through `municipality_numbers`. A second run with no upstream change
  writes nothing, and a test pins that.
- Each run applies in **one transaction**. A failure part-way leaves the register as it was.
- **Circuit breaker.** If a run would close more than 2% of active schools, or rename more than 5%,
  it aborts with `mass_change` and alerts rather than applying. This covers an upstream outage that
  returns an empty or truncated list, and an upstream schema change. A missing or renamed required
  field fails the run loudly, never becomes nulls. The run row is the alert source for the
  Alertmanager route decided on #3442.
- **Never hard-delete** a school or municipality. The only delete is a `held` row that loses a
  match review, since nothing can reference it.
- Operational: subscribe the operational address (`fau@ewb-solutions.as`) to Udir's
  `nxr-teknisk@udir.no` change notices, and watch status.kartverket.no for API changes.

### 5.4 Submission lookup (D9)

A "Mangler skolen din?" submission inserts a row into `register_lookups` in the same transaction
that creates the school and the submission. The runtime role may insert there and nowhere else in
the register. `fau register lookups` runs as `fau_register`. For each queued row it fetches NSR's
list for the submitted municipality (`/v4/enheter/kommune/{kommunenummer}`), then the detail of
each in-scope grunnskole it does not already hold:

- **Exactly one in-scope, active NSR school absent from the register has the same folded name**
  (the section 7 folding) as the submission. The submitted school absorbs it, exactly as a
  `possible_submission_match` resolved as "match" does: orgnr, attributes, `register_name` and
  slug. It becomes `verified`, the submission becomes `matched`, and Erik is told by email rather
  than asked. This is Erik's rule: the school is now known, so manual review has nothing to add.
- **A similar but not identical name** raises `possible_submission_match` for review, as in 4.5.
- **The school is already listed** in the register raises `submission_matches_listed_school`. The
  registrant missed it in the picker, and moving a tenant between school rows is a human's call.
- **Nothing found** leaves the submission in the ordinary review queue.

```sql
create table register_lookups (
  id            uuid        primary key,
  submission_id uuid        not null unique references school_submissions (id) on delete restrict,
  queued_at     timestamptz not null,
  processed_at  timestamptz,
  outcome       text        check (outcome in ('approved', 'review', 'not_found', 'failed')),
  attempts      integer     not null default 0
);
```

A failed fetch leaves the row queued and counts an attempt. After five attempts the outcome is
`failed`, and the submission waits for ordinary review.

## 6. Slug minting

ADR-002 already decides the frame: municipality segment `<kommunenr>-<navn>`, school segment by name
alone and unique within the municipality, `æ→ae`, `ø→oe`, `å→aa`, lowercase canonical, slug history
with 301s, and pretty slugs only after verification. This section fills the gaps.

**Algorithm** (one function in `crates/domain`, used for both municipalities and schools):

1. Take the input: the municipality's **Norwegian name** (D3), or the school's `display_name`.
2. Unicode NFC, then full lowercase.
3. Norwegian letters, per ADR-002: `æ→ae`, `ø→oe`, `å→aa`. Also `ä→ae` and `ö→oe`, their collation
   equivalents in Norwegian.
4. Letters with no decomposition: `đ→d`, `ŋ→n`, `ŧ→t`, `ß→ss`.
5. NFKD, then drop combining marks: `á→a`, `č→c`, `š→s`, `ž→z`, `ï→i`, `ü→u`, `é→e`, `à→a`.
6. Remove apostrophes without a separator ("Children's" gives `childrens`).
7. Replace every other run of non-`[a-z0-9]` characters with `-`, then collapse and trim the hyphens.
8. Cap the length at 80 characters, cut at a hyphen boundary.
9. Prefix the municipality slug with its four-digit number and a hyphen.

Examples, all from the real data:

- 3911 Færder → `3911-faerder`;
- 1515 and 1818 Herøy → `1515-heroey` and `1818-heroey`;
- 5540 Kåfjord → `5540-kaafjord`;
- 5610 Karasjok → `5610-karasjok`;
- "Grünerløkka skole" → `grunerloekka-skole`;
- "St. Svithun skole" → `st-svithun-skole`;
- "Deanu Sàmeskuvla" → `deanu-sameskuvla`;
- "Máze Skuvla/Masi skole Máze skole" → `maze-skuvla-masi-skole-maze-skole`, which argues for D5;
- "Hosle skole" in 3201 Bærum → `/fau/3201-baerum/hosle-skole`.

ADR-002's illustrative path used to be `3911-faerder/hosle-skole`, which paired a real school with the
wrong municipality. It was corrected on 24 September 2026, and tests should use real pairs.

**Collisions within a municipality.** There are none today *(measured)*. If one arises:

1. Append the post town slug from the school's visiting address (`-hosle`).
2. If that still collides, append `-2`, `-3`, and so on.
3. A slug held in history by a *different* school in the same municipality counts as taken, unless
   that school is closed. This follows ADR-002 rule 3: a closed school's slug may be re-issued, and
   the current holder wins.

**Reserved segments.** Municipality segments always begin with four digits and a hyphen, so they
cannot collide with any root path, locale code (#3439) or ADR-001 reservation. The existing test in
ADR-002 pins that. School segments sit under a municipality, so the only way to collide is for us to
author a route under `/fau/<kommune>/`. **Rule: no fixed route is ever authored at the school
level.** Forms such as "Mangler skolen din?" live on the municipality page itself, or post to
`/api/v1/`. A test enumerates the router and fails if any static route matches
`/fau/{segment}/{anything}`.

**Immutability and history.**

- A slug changes only when the Norwegian municipality name or the school's `display_name` changes,
  and then only if the new slug differs. A case-only change in NSR changes nothing.
- The old slug moves to `*_slug_history` with its validity period, and requests 301 to the
  canonical path.
- A renumber changes the municipality slug, and with it every school path in that municipality. No
  school rows change: the school slug history stays keyed on the municipality UUID, so resolution
  runs municipality first, then school.
- Resolution follows ADR-002 exactly: current holder, then a single historical holder (301), then
  404. If two historical holders share a slug, the result is 404, never a guess.

## 7. The picker

**Shape.** The flow asks for municipality first, then school. The data makes that cheap: 358
municipalities, and at most 158 schools in one (Oslo). So there is no need for fuzzy school search
across the whole country.

```
GET /api/v1/register/kommuner?q=<text>          → up to 10 municipalities
    [{ id, number, name, official_name, county_name, path }]

GET /api/v1/register/kommuner/{id}/skoler       → every pickable school in it, unpaged
    [{ id, display_name, grade_from, grade_to, ownership, post_town, has_active_fau, path }]
    The client filters as the user types; the list is at most a few hundred rows.

GET /api/v1/register/skoler/{id}                → one school, for /bli-med prefill
```

"Pickable" means all of the following:

- `status = 'active'`;
- `verification in ('listed', 'verified')`;
- effectively in scope (`coalesce(scope_override, in_scope)`).

`has_active_fau` is already public, since claimed pages are indexable (ADR-002). Selecting such a
school leads straight to the §3.2 path. A *pending* FAU is **not** exposed here; it surfaces only at
submit, as §3.2 specifies. "Mangler skolen din?" sits under the school list.

**Matching.** Normalisation happens in Rust at write time, into `search_text`, and at query time on
`q`. It is not done in the database, so it does not depend on the database's collation or `LC_CTYPE`.
`search_text` is the space-joined set of:

- each name folded (NFC and lowercase);
- the same through the slug transliteration (`tromsoe`, `aalesund`);
- a lossy variant (`tromso`, `alesund`), because that is what people type.

For municipalities, that covers every official name in `municipality_names`, so "karasjok" and
"kárášjohka" both find 5610.

Query order:

1. a word-prefix match (`search_text like '% ' || $q || '%'` over a leading-space form);
2. then `pg_trgm` similarity for typos, with `similarity > 0.3`;
3. ranked exact prefix first, then word prefix, then similarity.

`pg_trgm` is a trusted extension since PostgreSQL 13, so `fau_migrate` can create it with its
existing `create` right on the database. At 358 and about 2,700 rows a sequential scan is
sub-millisecond, so a GIN `gin_trgm_ops` index is optional. `unaccent` is not needed.

**Ordering with no Latin-collation assumption (#3439).** The database never sorts for display.
Results are sorted in the application with an ICU collator for the request locale: ICU4X in Rust,
or `Intl.Collator` in the browser. So Norwegian puts æ, ø and å after z, and Northern Sámi or English
sort by their own rules when those locales arrive. Column collations stay the deterministic default.
Nothing may `order by` a name column for presentation, and a test pins that Bokmål ordering places
"Ålesund" after "Øygarden".

**Prefill from `/bli-med/<uuid>`** (flow §3.1; ADR-002 §2):

- It pre-selects the school and its municipality in the form. The user can change both, and the
  form calls it "Forslag" rather than fact.
- The FAU name defaults to the linked Brreg entity's case-normalised registered name, or else to
  `display_name` (4.6).
- It grants nothing and carries nothing but the school UUID.
- An unknown or closed UUID lands on the ordinary picker with a neutral message. It never errors
  in a way that distinguishes "never existed" from "closed", beyond the public 410 page.

## 8. Public exposure and enumeration

**What public pages and outreach links may show:**

- school `display_name`;
- municipality and county;
- visiting address, meaning the institution's address and not a person's;
- website, rendered as an external link with `rel="nofollow noopener noreferrer"` and never fetched
  by us. A bare `www.…` gets `https://`, and anything that is not http(s) is dropped;
- grade span ("1.–7. trinn");
- public or private;
- whether an FAU is active on the portal (ADR-002's claimed or unclaimed states);
- a provenance line: "Kilde: Nasjonalt skoleregister (Utdanningsdirektoratet), sist oppdatert
  <dato>";
- for a verified submission, ADR-002's decision provenance ("Opprettet etter vedtak i …").

**Never shown:**

- pupil and staff counts. They are public in NSR but useless here and invite comparison;
- the organisation number, which is harmless but serves no user;
- anything about who created or runs an FAU;
- pending state;
- unverified submissions outside their own `/s/<uuid>`;
- any contact detail.

**Attribution.** A site-wide "Datakilder" note, linked from every register-backed page, reads:

- «Inneholder data under Norsk lisens for offentlige data (NLOD) tilgjengeliggjort av
  Utdanningsdirektoratet»;
- «Inneholder data under Norsk lisens for offentlige data (NLOD) tilgjengeliggjort av
  Brønnøysundregistrene»;
- «Kommunedata: © Kartverket, CC BY 4.0» and «Statistisk sentralbyrå, CC BY 4.0»;
- a note that display names may be adjusted by us, as NLOD §5 requires when data is changed.

**Enumeration: what #3441's criterion can and cannot mean.** #3441 says a per-school link must not
let anyone "enumerate the register". Taken literally, that conflicts with the approved design three
ways:

- the picker must list every school in a municipality;
- ADR-002's municipality page lists schools;
- the whole register is Udir's open data, downloadable anonymously in 19 requests.

Hiding it would protect nothing. What enumeration could actually harm, and what the design protects,
is:

1. **Personal data.** None exists in the register. Submitters live in `school_submissions`, which no
   public endpoint reads.
2. **Unverified submissions.** User-typed names are an abuse vector (offensive or fake names). They
   appear only at their own `/s/<uuid>` and are never listed (D7).
3. **Pending registrations.** They are never listed. They surface only to someone who tries to
   register the same school, which §3.2 accepts.
4. **Membership.** No register endpoint knows anything about members (flow principle 7).
5. **Outreach links.** They carry only a school UUID that is already public on the school page's
   canonical link. So they are **not secrets and must never be treated as one**. Their safety comes
   from granting nothing: the §3.2 duplicate block and the unique index apply whatever the entry
   point, so a link cannot claim an existing FAU. No per-recipient token or identifier is ever added.
   One campaign-level parameter shared by every recipient of a mailing, such as `?kampanje=2026-10`,
   is allowed (D10).

D6 asks Erik to confirm this reading.

**Rate limits** (starting values, to tune):

- Search and list endpoints: 60 requests per minute per IP with a burst of 20, returning 429. Set
  coarse at ingress-nginx (`limit-rpm` annotation, already deployed) and per IP in the application.
- The municipality list is effectively static. Serve it with a long `Cache-Control` and an ETag so
  the picker is one cached request.
- "Mangler skolen din?": per account and per municipality, as ADR-002 requires, starting at 3 per
  account per day and 5 per municipality per day. Every submission also emails Erik.
- Public pages: the general page limit. They are cacheable because they hold no session data.

## 9. #3431, the prospect register: one truth, two uses

#3431 builds a CSV of municipality, school, FAU, stable ID, official URLs, and a published contact
channel with source, date and status. It is sales material, may contain personal data (a named FAU
leader's published address), and lives **outside the product database**. To avoid two truths:

- **The product register is the only source of school identity.** #3431 keys every row on our school
  UUID and orgnr, taken from an export of the product register: `fau register export`, which lists
  UUID, orgnr, municipality number, name, path and the linked FAU's Brreg orgnr. It never edits
  school data.
- **FAU contact e-mail is fetched live from Brreg by #3431 at send time,** by the FAU's orgnr, and
  never stored in the product (Erik, 24 September). The register's parser does not read Brreg's
  `epostadresse`, `mobil` or `telefon` fields at all.
- **The flow is one-way.** Nothing from #3431 is imported into the product. If sales finds a school
  NSR lacks, it goes through "Mangler skolen din?" or the operator path like any other.
- Outreach links are built from the export as `/bli-med/<uuid>`. Contact data stays in #3431, under
  #3426 and #3427's retention and authorisation, and outreach remains unauthorised until Erik says
  otherwise.

## 10. Testing, the backbone

Tests run against real PostgreSQL, as in the existing harness. Every NSR, Kartverket and SSB response
comes from **recorded fixtures** captured from the live APIs. They are public data, so they can be
committed. CI never calls the network.

**Slugs** (table-driven unit tests in `crates/domain`):

- every example in section 6;
- Herøy twice;
- Våle and Re sharing 0716 in different eras;
- Sámi names (Gáivuotna, Kárášjohka, Guovdageaidnu, Aarborte);
- punctuation cases ("Children's", "Fossen skole 1.-4. skole", "Viti skole avd Nordbyhagen Sone 1, 2
  & 3");
- the 80-character cap;
- idempotence: slugging a slug returns it unchanged.

**Import and sync:**

- A seed from fixtures creates exactly the expected in-scope set. Adult education, VGS-only, 2599 and
  inactive rows are excluded; combined and special schools are included.
- A second run with identical fixtures writes nothing (`no_change`).
- A rename:
  - creates a new slug and a history row, and the old path 301s;
  - a case-only rename changes nothing;
  - a curated `display_name` is not overwritten.
- Municipality renumber: 3024 to 3201 Bærum from the SSB fixture moves the slug, and old school paths
  301 through the municipality history.
- Split: 1507 into 1508 and 1580 raises `municipality_split_or_merge` and touches no school until it
  is resolved. After resolution, `/fau/1507-aalesund/<Haram school>` 301s to 1580.
- Boundary adjustment: the 2026 change from 3118 to 3207 and 3216 is ignored.
- Official-name-only change: "Oslo" to "Oslo - Oslove" changes no slug.
- Closure without an FAU closes the row. Closure with an FAU closes the row, raises
  `closure_with_fau`, keeps the tenant working, and the public page returns 410.
- Re-registration from the real Stange ungdomsskole pair `975270920` and `933181995`:
  - applied automatically when no FAU references the old row;
  - held for review when one does;
  - "same school" keeps the UUID and records both orgnrs.
- A submitted school, then an NSR row with a similar name in the same municipality: the NSR row is
  held and not pickable. "Match" keeps the submitted UUID and tenant, and issues the slug.
- The circuit breaker aborts on a truncated list (an empty page) and on a mass rename.
- An unknown municipality number skips the row and raises an item.
- A missing required field fails the run rather than writing nulls.
- Deleting a referenced school or municipality fails on the foreign key.

**One FAU per school:**

- a second pending or active tenant for the same school fails;
- a closed one does not block;
- two concurrent transactions produce exactly one tenant, and the loser gets the §3.2 message, not a
  500.

**Picker and exposure:**

- "tromso", "tromsoe", "Tromsø" and "TROMSØ" all find 5501;
- "karasjok" and "kárášjohka" both find 5610;
- "aal" finds Ålesund;
- a typo ("bearum") finds Bærum;
- Bokmål ordering puts Ø before Å and both after Z;
- pending, held, rejected, closed and out-of-scope schools never appear in the picker or on the
  municipality page;
- the school list endpoint returns no field outside section 8's list, pinned by a response-schema
  snapshot;
- `/bli-med/<uuid>` for a school with an active FAU offers the §3.2 relay, not creation;
- `/bli-med/<uuid>` grants no session, membership or role;
- rate limits return 429;
- the attribution text is present;
- no static route exists under `/fau/{m}/`.

**Privileges:** per D9, the runtime role cannot update register tables. The schema-review tests
extend to the new tables unchanged: no forbidden column names, and the global tables carry no
`tenant_id`.

## 11. Implementation split

**#3441 (this card):**

- the register migration: tables, the `tenants.school_id` foreign key and the one-live-FAU index;
- the `fau register sync` and `export` subcommands, with fixtures;
- slug minting and the resolver function in `crates/domain`;
- search query functions in `crates/persistence`;
- the review-item model, plus a minimal `fau register review list|resolve` CLI for Erik (D11);
- Brreg FAU entities and their matching (4.6);
- the submission lookup queue and `fau register lookups` (5.4);
- the attribution text;
- the seed run on each environment, with Erik's go-ahead.

**#3417 (signup):**

- the picker endpoints and the form wiring;
- `/bli-med/<uuid>` prefill;
- the "Mangler skolen din?" submission, which creates a school and a submission in one transaction
  with the ADR-002 fetch, ingest and notification rules; ingest itself stays with #3447;
- mapping the unique violation to §3.2;
- search rate limits.

**#3418 (invitations and lifecycle):**

- nothing structural;
- optionally, school setup may suggest initial cohorts from `grade_from`/`grade_to` ("1.–7. trinn"),
  which is a suggestion, not a rule.

**Elsewhere:**

- #3423 and #3422: the public `/fau`, `/s` and `/bli-med` pages, 301 and 410 handling, `noindex`
  and the screens;
- #3424: the CronJob manifest, the egress policy and the D9 role;
- #3426: retention of `school_submissions` and archived decision documents;
- #3421: system-level audit events (5.2 step 6);
- #3431: consumes `fau register export`.
- #3498: merging FAU-er, sharing the old FAU-er's documents into a successor FAU (4.5).
- #3499: the D11(b) admin review screen, high priority after the MVP CLI.

## 12. Decisions

Erik answered on #3441 on 24 September 2026:
- **D1:** (a) and (b) combined, with Brreg's FAU entities matched to schools by address (2.5, 4.6).
- **D2:** (a).
- **D3:** (a).
- **D4:** (a).
- **D5:** (b).
- **D6:** (a).
- **D7:** (a).
- **D8:** (a).
- **D9:** (a), but weekly, plus an immediate NSR lookup on each submission (5.4).
- **D10:** (b).
- **D11:** (a), with (b) high priority after it.

He also asked for FAU-er to be mergeable when schools merge (4.5). The options as they were put to
him follow, unchanged.

**D1. Data sources.**

- Options:
  - (a) NSR v4 for schools, Kartverket for current municipalities, SSB Klass for municipality code
    history;
  - (b) Brreg's Enhetsregisteret directly, filtered on NACE codes;
  - (c) #3431's hand-built list.
- **Recommend (a)**, and subscribe `fau@ewb-solutions.as` to Udir's `nxr-teknisk@udir.no` notices.
- Consequence:
  - all three sources are Norwegian public bodies, open (NLOD or CC BY 4.0) and anonymous;
  - nothing new for the supplier list;
  - attribution is required;
  - (b) loses NSR's school categories and grade levels, and (c) rots.

**D2. Which schools are in scope.**

- Options:
  - (a) the section 2.3 filter: active grunnskoler, public and private, combined and special
    included; adult education, VGS-only and abroad excluded; Longyearbyen included under a manual
    "2100 Svalbard" entry. 2,677 schools today, with an operator override per school;
  - (b) the same, but also require a pupil count and grade levels, which excludes new schools such
    as Fjellnær friskule until Udir fills them in;
  - (c) every active `ErGrunnskole` row, 2,733.
- **Recommend (a).**
- Consequence: a handful of non-schools remain pickable ("Porsgrunn kommune Vikarer") until someone
  marks them out of scope. The alternatives either hide real new schools (b) or list adult education
  (c). Svalbard's inclusion rests on an unverified legal assumption.

**D3. Which name the municipality slug uses.**

- Options:
  - (a) the Norwegian name (Kartverket `kommunenavnNorsk`), so `5540-kaafjord` and `0301-oslo`;
  - (b) the priority-1 official name, so `5540-gaivuotna`;
  - (c) SSB's full official form, so `0301-oslo-oslove`.
- **Recommend (a)** for the slug, and display the full official name on the municipality page and in
  search.
- Consequence:
  - with (a), URLs stay stable through official-name changes like Oslo's in 2026;
  - (b) and (c) put Sámi forms in URLs, which is respectful but unreadable to most searchers, and (c)
    would have 301'd every Oslo URL on 1 January 2026;
  - either way, search matches every official name.

**D4. Transliteration beyond æ, ø and å.**

- Options:
  - (a) section 6's rules: ä→ae, ö→oe, strip other diacritics, đ→d, ŋ→n, ŧ→t;
  - (b) German-style ü→ue as well.
- **Recommend (a).**
- Consequence: an ADR-002 amendment of one line. Low stakes, and it touches only 4 of 2,677
  school names today, plus the Sámi municipality names if D3 is (b).

**D5. Curating display names.**

- Options:
  - (a) render NSR's name as is;
  - (b) allow an operator override (`display_name_curated`) that the sync then respects;
  - (c) (b) plus automatic cleanup rules, such as stripping a leading "<Kommune> kommune".
- **Recommend (b).**
- Consequence:
  - 41 names carry a municipality prefix and 128 an "avd" suffix, so some URLs look clumsy until
    curated;
  - curation needs the NLOD "changed by us" note;
  - automatic rules (c) would silently mangle names such as "Elverum kommune - Ydalir skole".

**D6. How to read "must not allow the register to be enumerated".**

- Options:
  - (a) as section 8 does: the register is public open data and may be listed; protect personal
    data, unverified submissions, pending state and membership, and treat outreach links as
    non-secret;
  - (b) literally: no complete listing, search-only with minimum query lengths.
- **Recommend (a).**
- Consequence:
  - (b) contradicts the approved picker and ADR-002's municipality pages, and protects data anyone
    can download from Udir;
  - (a) needs Erik to confirm the card's criterion is met in this sense, recorded in
    planning-decisions.

**D7. Unverified submitted schools in the picker.**

- Options:
  - (a) hidden until verified;
  - (b) shown in the picker marked "ikke verifisert".
- **Recommend (a).**
- Consequence:
  - with (a), a second person looking for the same unlisted school during the review week submits it
    again, and the one-FAU rule cannot catch that because they are two rows. Erik resolves it in
    review, where the second submission shows as a near-duplicate;
  - (b) prevents that, but publishes user-typed names, an abuse surface, before a human has seen
    them.

**D8. Automation around school closures and re-registrations.**

- Options:
  - (a) automatic when no FAU is attached, and a review item when one is (section 4.5);
  - (b) always review;
  - (c) always automatic.
- **Recommend (a).**
- Consequence:
  - (b) puts dozens of routine closures a year in Erik's queue;
  - (c) could move or orphan a live FAU on a heuristic;
  - with (a), Erik sees only cases where a real FAU is affected.

**D9. Who runs the sync, and with which database role.**

- Options:
  - (a) a daily CronJob plus operator-run seed, with a dedicated `fau_register` role that alone may
    write register tables, while `fau_app` gets `select` on them and `insert` for submissions only;
  - (b) the same schedule reusing `fau_app`;
  - (c) no CronJob, an operator runs the sync by hand monthly.
- **Recommend (a).**
- Consequence:
  - (a) adds one role to `roles.sql` and to #3424's provisioning, and keeps a compromised web
    process from rewriting the register;
  - (b) is simpler but breaks the least-privilege line 0002 draws;
  - (c) lets closures and new schools lag by weeks.

**D10. Tracking on outreach links.**

- Options:
  - (a) no parameters at all;
  - (b) one campaign-level parameter shared by every recipient of a mailing (`?kampanje=2026-10`),
    never per-recipient;
  - (c) per-recipient tokens.
- **Recommend (b).**
- Consequence:
  - (b) measures whether outreach works without personal data in the URL;
  - (c) makes the link carry personal data and contradicts #3441;
  - whichever is chosen, outreach itself stays unauthorised until #3426 and #3427 clear it.

**D11. Review tooling for the register queue.**

- Options:
  - (a) a `fau register review` CLI run by the operator or the agent for the MVP, with every new item
    emailed to `fau@ewb-solutions.as`;
  - (b) an admin web screen now.
- **Recommend (a).**
- Consequence: no admin UI on the MVP path. This also answers flow §12's open item on
  unverified-school review tooling for now.

## 13. Sources

All checked 23 September 2026.

- NSR API v4 specification: https://data-nsr.udir.no/swagger/v4/swagger.json (UI:
  https://data-nsr.udir.no/swagger/index.html)
- NSR API endpoints used: https://data-nsr.udir.no/v4/enheter,
  https://data-nsr.udir.no/v4/enhet/974552124, https://data-nsr.udir.no/v4/enheter/endretetter,
  https://data-nsr.udir.no/v4/skolekategorier, https://data-nsr.udir.no/v4/utgaattyper,
  https://data-nsr.udir.no/v4/naeringskoder
- NSR API v3 specification: https://data-nsr.udir.no/swagger/v3/swagger.json
- NSR in Felles datakatalog (licence, frequency, access):
  https://data.norge.no/datasets/d8431635-2ae6-40af-b0ec-8869a2fa3f89
- NSR public site: https://nsr.udir.no (a JavaScript application; its "om" and "hjelp" pages could
  not be read without a browser, so **their content is unverified**)
- NLOD 2.0 licence text, §5 attribution: https://data.norge.no/nlod/no/2.0
- Kartverket Administrative enheter API: https://api.kartverket.no/kommuneinfo/v1 and
  https://ws.geonorge.no/kommuneinfo/v1/openapi.json
- Kartverket metadata and licence (CC BY 4.0):
  https://kartkatalog.geonorge.no/metadata/administrative-enheter-kommuner/041f1e6e-bdbc-4091-b48f-8a5990f3cc5b
- SSB Klass classification 131 and its change feed:
  https://data.ssb.no/api/klass/v1/classifications/131,
  https://data.ssb.no/api/klass/v1/classifications/131/changes?from=2023-12-01&to=2024-01-31,
  https://data.ssb.no/api/klass/v1/api-guide.html
- SSB API licence (CC BY 4.0): https://www.ssb.no/api
- Opplæringslova 2023 § 10-5: https://lovdata.no/dokument/NL/lov/2023-06-09-30/KAPITTEL_10
- Privatskolelova § 5A-5: https://lovdata.no/dokument/NL/lov/2003-07-04-84/KAPITTEL_5
- Project documents: docs/url-scheme.md (ADR-002), docs/fau-creation-and-membership-flow.md,
  docs/planning-decisions.md, docs/localisation-design.md (#3439),
  docs/internal-supplier-ownership.md, backend/migrations/0002_identity_and_tenancy.sql,
  backend/crates/app/tests/schema_review.rs, backend/db/roles.sql
- Favro: #3441 (card `24ead1b77a3f324a3eba8a87`, including the 9 September comment on slugs) and
  #3431 (card `ec72098642549d91831c1b6b`)

**Unverified, collected in one place:**

- NSR rate limits;
- the content of nsr.udir.no's information pages;
- whether FAU rules apply on Svalbard;
- whether PostgreSQL's `pg_trgm` behaves identically on the production CloudNativePG image. The design
  avoids depending on it by normalising in Rust.
