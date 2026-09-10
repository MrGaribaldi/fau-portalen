# ADR-001: Rust/TypeScript-repo og containerkontrakt

Dato: 7. september 2026. Eier: Systemarkitektur og datamodell. Favro: #3411.
Status: Forslag til review. Dette dokumentet spesifiserer arbeid for #3416, #3422 og #3424; det er ikke en implementasjon eller en utrullingstillatelse.

## Grunnlag og beslutningsstatus

Godkjente premisser er Rust-backend, TypeScript med et lett frontendrammeverk, PostgreSQL, Bokmål, og det eksisterende infra-tools-oppsettet med tre servere. Tenant-, rolle- og historikkmodellen i #3412 er godkjent, inkludert fullstendige snapshots samlet per redigeringsøkt. Senere svar i [planning-decisions.md](planning-decisions.md) overstyrer [prosjektgrunnlaget](../prosjektgrunnlag.md).

[Infra-kartleggingen](infra-mvp-mapping.md) rapporterer `serve`, port 8000, ConfigMap/Secret, PostgreSQL, RabbitMQ og OTLP fra den tidligere kildegjennomgangen. Rettelse etter videre inspeksjon 7. september: infra-tools-checkouten finnes lokalt på commit `6c301e15e3ee83a9797971046ffe1bc9b8593ca3`; den ble oversett av det første filsøket. Eksakte appvariabler, collector-endepunkt og Flux-rekkefølge er fortsatt ikke kontrollert på nytt i denne ADR-en. #3406 står fortsatt til gjennomgang. Kontrakten nedenfor er et konkret mål for tilpasningen, ikke en påstand om direkte kompatibilitet uten endringer.

Ny føring: Erik ønsker å undersøke eksisterende dedikert server som rimelig start, med senere flytting til andre noder. Det åpner for en annen fysisk topologi enn tre cloud-servere; endelig valg er uavklart. [Vurderingen](existing-server-assessment.md) beskriver VM-alternativ, hybridalternativ og nødvendig lagringstilpasning. Appimage, HTTP-kontrakt og separate migreringer beholdes i forslaget uansett fysisk plassering.

Nye forslag til godkjenning er ett applikasjonsrepo, én produksjonsimage med ferdigbygd frontend, en modulær Rust-applikasjon, eget migreringssteg og driftskontrakten nedenfor. Frontendrammeverk velges i #3415. Rust HTTP-/databasebibliotek og eksakte verktøyversjoner spesifiseres ved implementasjon og dokumenteres med lockfiler; denne ADR-en velger ingen uavklart leverandør eller identitetstjeneste.

## Repository og bygg

Foreslått struktur i et eget applikasjonsrepo; eksisterende infra-tools gjenbrukes gjennom versjonsfestede modulreferanser, uten å flytte eller kopiere hele infrastrukturen:

```text
backend/
  Cargo.toml, Cargo.lock
  crates/app/                 binær med serve og migrate
  crates/domain/              identitet, tenant, roller, dokument, lease, audit
  crates/persistence/         PostgreSQL og transaksjoner
  migrations/                ordnede, uforanderlige SQL-migreringer
  tests/                     integrasjon mot virkelig PostgreSQL
frontend/
  package.json, <lockfil>
  src/                       Bokmål-landingsside og app
  dist/                      generert bygg, ikke håndredigert
api/openapi.yaml              versjonsfestet API-kontrakt
Dockerfile
.dockerignore
compose.yaml
.env.example                 bare lokale eksempelverdier
rust-toolchain.toml
infrastructure/              prosjekttilpasning og modulversjoner
gitops/                      app, Service, konfigurasjon, migreringsjobb
docs/                        ADR-er og driftsoppskrifter
```

Domene skal ikke avhenge av HTTP eller UI. Persistence eier transaksjonsgrensene som kobler arbeidskopi, redigeringssegment, ferdig versjon, audit og command receipt fra [#3412](tenant-role-history-design.md). Konto er global; tenantkontekst og autorisasjon håndheves per operasjon på serveren. Leases, sesjoner og varig arbeid skal ikke være bundet til én pod eller lokalt filsystem.

API-kontrakten og frontendens genererte typer valideres sammen i CI. Genereringsverktøy velges med #3415/#3416. Node brukes til frontendbygg; produksjonsimage inneholder bare Rust-binær, nødvendige runtimebiblioteker/CA-sertifikater, migreringer og statiske frontendfiler. Byggesteg bruker festede toolchains/baseimages og lockfiler. Ingen hemmeligheter eller miljøspesifikke produksjonsadresser bygges inn i frontend. Samme image-digest brukes i test, migrering og deploy.

## HTTP og prosesskontrakt

| Grensesnitt | Foreslått kontrakt |
| --- | --- |
| Entrypoint | Rust-binær `/app/fau`; exec-form, direkte signalhåndtering |
| Standardargument | `serve`; starter HTTP på `0.0.0.0:8000` |
| Migreringsargument | `migrate`; utfører ventende migreringer og avslutter med 0 ved suksess, ikke-null ved feil |
| Versjon | `--version`; byggerevisjon uten konfigurasjon eller hemmeligheter |
| Nettleser | `/` landingsside, `/app/` applikasjon, `/assets/` byggfiler |
| API | `/api/v1/`; JSON-feil med stabil feilkode og request-ID |
| Helsesjekker | `/health/live` og `/health/ready`; minimale svar uten interne adresser |

Frontend og API får samme origin. Klientruter under `/app/` kan falle tilbake til appens HTML ved navigasjon. Ukjente API-ruter og manglende assets returnerer 404, aldri HTML-fallback. Hashnavngitte assets kan caches lenge; HTML skal revalideres. Bevar forrige releases assets gjennom en overlappende utrulling eller bruk en kontrollert utrullingsstrategi som hindrer at ny HTML treffer gamle pods uten filene. #3424 skal teste dette eksplisitt.

TLS avsluttes ved ingress i foreslått deploymodell. Appen stoler bare på videresendte headere fra konfigurert proxy. Offentlig origin kommer fra konfigurasjon, ikke vilkårlig Host-header. Sesjonscookies, CSRF-beskyttelse og magic-link-detaljer konkretiseres i #3414/#3417. API krever medlemskap og rettigheter uavhengig av hvilken organisasjon frontend viser.

SIGTERM setter readiness til feil, stanser nytt arbeid og gir pågående requests/transaksjoner inntil 25 sekunder til å fullføre før prosessen avslutter. Kubernetes grace period foreslås til minst 35 sekunder, inkludert eventuell ingress-drenering; tidsverdiene skal verifiseres under last. Avbrutte transaksjoner skal ikke bekreftes til klienten. Restart må ikke miste bekreftet autosave; segmenter og utløpte leases gjenopptas/finaliseres idempotent fra databasen.

Runtime kjører uten root, med skrivebeskyttet rotfilsystem og midlertidig `/tmp` ved behov. Ingen dokumenter eller permanente tilstander lagres i containeren. Kubernetes Service peker til port 8000. Resource requests/limits må måles i #3408/#3424, ikke utledes av dette dokumentet.

## Miljøvariabler og hemmeligheter

Navnene nedenfor er foreslått appkontrakt. #3424 må mappe infra-tools sine faktiske ConfigMap-/Secret-nøkler til disse eksplisitt; Python-/Django-spesifikke variabler skal ikke kopieres som Rust-konfigurasjon.

| Variabel | Bruk og validering |
| --- | --- |
| `APP_ENV` | `development`, `test` eller `production`; produksjon avviser dev-adaptere |
| `HTTP_BIND` | Standard `0.0.0.0:8000` |
| `PUBLIC_BASE_URL` | Påkrevd; HTTPS i produksjon, eksplisitt localhost tillatt lokalt |
| `DATABASE_URL` | Secret; runtimekonto med begrensede rettigheter, aldri logget |
| `MIGRATION_DATABASE_URL` | Secret kun i migreringsjobb; DDL-konto, ikke tilgjengelig i serve-pod |
| `DB_POOL_MAX_CONNECTIONS` | Positivt heltall; samlet antall per replika/jobb må passe DB-budsjett |
| `LOG_LEVEL` | Standard `info`; strukturerte logger til stdout |
| `OTEL_SERVICE_NAME` | Stabilt tjenestenavn, foreslått `fau-app` |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Miljøets collector-endepunkt; avklar protokoll med infra-tools |
| `OTEL_EXPORTER_OTLP_PROTOCOL` | Eksplisitt eksportprotokoll som valgt Rust-SDK støtter |
| `MAIL_TRANSPORT` | Lokal testadapter eller senere valgt produksjonsadapter |
| `SIGNUP_NOTIFICATION_TO` | `fau@ewb-solutions.as` i produksjon; lokal testmottaker i Compose |

Eventuelle OTLP-headers, mailnøkler, sesjonsnøkler, RabbitMQ- og S3-legitimasjon er Secrets med navn fastsatt når adapteren velges. De skal ikke ligge i image, Git, byggargumenter eller nettleserkonfigurasjon. Migreringsprosessen krever bare sin databasekonfigurasjon, ikke e-post eller offentlig origin. Ugyldig obligatorisk konfigurasjon gir tydelig feilmelding med variabelnavn, uten verdi, og ikke-null exit. Midlertidig DB-nedetid under serve gir readiness-feil og avgrenset retry, ikke en konfigurasjonsfeil.

RabbitMQ beholdes i infrastrukturen. Denne ADR-en gjør det ikke til en ny synkron avhengighet for dokumentlagring. Varig e-post-outbox og eventuell RabbitMQ-arbeider må spesifiseres sammen med #3410/#3417: levering med retry, idempotens og håndtering av varig feil kreves før signup settes i drift. Separat worker kan bruke samme image dersom valgt løsning trenger det. S3-originaler er valgfrie i MVP; en deaktivert S3-adapter skal ikke hindre tekstbasert appstart.

## Migrering og utrulling

`serve` utfører aldri DDL. Én releasejobb kjører `migrate` mot samme database med egen konto. Migreringsverktøyet må holde en databasebasert lås gjennom hele migreringen, registrere versjon/sjekksum og ha avgrenset låseventetid. To samtidige jobber skal ikke anvende samme migrering parallelt. Endret sjekksum for anvendt migrering er feil.

Hver migrering er transaksjonell der SQL-operasjonene tillater det. Eventuelle ikke-transaksjonelle steg krever særskilt gjenopptakingsprosedyre før release. Runtimekontoen skal ikke ha generell DDL eller endringsrett på ferdigstilt historikk/audit; kontrollerte slette-/vedlikeholdsrutiner eies av #3426/#3421.

Foreslått releasefølge: bygg og kontroller image → kjør migreringsjobb → vent på bekreftet suksess → rull ut app med samme digest → kontroller readiness og smoke-scenarioer. #3424 må implementere en faktisk Flux-/pipelinebarriere; YAML-filrekkefølge er ikke bevis på sekvens. Ved migreringsfeil stoppes ny apputrulling og eksisterende app beholdes.

Schemaendringer følger expand/contract: migreringen må støtte både gammel og ny app gjennom utrullingen. Destruktiv opprydding kommer først i senere release når gammel kode er ute. Appen erklærer og sjekker støttet schemakontrakt ved oppstart; nyere additive migreringer må kunne være kompatible. Rollback bytter appimage bare hvis schema fortsatt støttes. Ingen automatisk nedmigrering eller database-restore som ordinær rollback. Backup og isolert restore er eget evidenskrav i #3425.

## Healthchecks og observabilitet

Liveness undersøker at HTTP-prosessen svarer; database, mail, RabbitMQ, S3 og OTLP skal ikke være liveness-avhengigheter. Readiness krever ferdig lokal initialisering, lesbare frontendfiler, en kort DB-sjekk og kompatibelt schema. Forslag: DB-sjekk med ett sekund timeout; feil gir 503. Probe kan bruke et kort cachet resultat for å unngå unødig DB-last. Startup-probe bruker liveness inntil serveren er startet. Kubernetes skiller restart ved liveness/startup-feil fra trafikkstyring ved readiness-feil. [Kilde: Kubernetes probes](https://kubernetes.io/docs/tasks/configure-pod-container/configure-liveness-readiness-startup-probes/).

Startverdier for review: startup hvert 2. sekund, 30 forsøk; liveness hvert 10. sekund, timeout 2 sekunder, 3 feil; readiness hvert 5. sekund, timeout 2 sekunder, 2 feil. Mål oppstarts- og lastadferd før disse verdiene behandles som produksjonsinnstillinger. Bortfall av mail/telemetri skal vises som degradert funksjon og egne alarmer, uten å ta dokumentarbeid ut av drift.

JSON-logger inneholder tidspunkt, nivå, tjenesteversjon, request-ID, rutemal, status og varighet. Ikke logg URL-query, e-postadresser, cookies, Authorization, magic-link-token, SQL-parametre eller dokumentinnhold. Bruk rutemal fremfor rå URL også i tracing. Forretningsaudit ligger transaksjonelt i PostgreSQL og har eget tilgangs-/oppbevaringsregime; tap av driftslogger må ikke påvirke sporbarheten.

OTLP er en ekstra eksportkanal med avgrenset kø og timeout; collector-feil skal ikke stoppe requests eller vokse minnet ubegrenset. Velg én tydelig loggtransport for å unngå dobbeltinnsamling via stdout og OTLP. Standardiserte OTEL-navn er utgangspunkt, men støtten må kontrolleres i valgt Rust-SDK; alle språkimplementasjoner støtter ikke nødvendigvis hele miljøvariabelsettet. [Kilde: OpenTelemetry environment variables](https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/).

## Lokal Compose-kontrakt

Standardoppsettet har `db`, engangstjenesten `migrate`, `app` og en lokal mail-testadapter uten ekstern levering. PostgreSQL-major skal samsvare med verifisert produksjonsmål; kartleggingens demo bruker 17.5, men patchversjon/digest må velges og festes ved implementasjon. Databasevolum er lokalt og vedvarende. Ingen eksterne credentials eller produksjonsendepunkter kopieres inn.

`db` må være healthy før `migrate`; `app` starter først etter at migreringen har avsluttet vellykket. Compose har egne betingelser for `service_healthy` og `service_completed_successfully`; startrekkefølge alene bekrefter ikke klar database. [Kilde: Docker Compose startup order](https://docs.docker.com/compose/how-tos/startup-order/).

Målet for #3416 er at en ny checkout kan startes med kopiering av `.env.example` til ignorert `.env`, deretter `docker compose up --build`, og åpnes på `http://localhost:8000`. Dette er en fremtidig akseptansekommando, ikke kjørbar funksjonalitet i dette arbeidsområdet ennå. Dokumenter også migrering ved oppgradering av eksisterende Compose-volum; ikke stol på at en tidligere fullført engangscontainer automatisk utfører nye migreringer. En valgfri utviklingsprofil kan tilby TypeScript-devserver som proxyer API til Rust. RabbitMQ og OTLP-collector kan legges i lokale integrasjonsprofiler når adaptere er klare; produksjonsoppsettet beholdes.

## Alternativer og avveininger

| Alternativ | Vurdering |
| --- | --- |
| Én image med frontend | Anbefalt for MVP: felles release og origin, færre appkomponenter. Frontendendring krever ny samlet image; assets må håndteres under utrulling. |
| Separate frontend-/backendimages | Mulig senere ved egne skalering-/releasebehov; gir flere utrullings- og kompatibilitetsgrenser nå. |
| Migrering ved start av hver replika | Avvist i forslaget: blander DDL og request-runtime, krever høyere rettigheter i alle pods og vanskeliggjør releasebarrieren. |
| Kubernetes-only utvikling | Avvist i forslaget: lokal Compose gir et mindre oppsett for appkontrakten; Kubernetes-egenskaper må fortsatt testes separat. |

## Akseptanse og overlevering

Dokumentkontroll gjennomført mot kortets krav og siste lokale beslutninger: repostruktur, én appimage, port/serve, miljøvariabler, migrering, healthchecks, logging/OTLP og Compose er spesifisert. Nye valg er merket som forslag. Ingen bygg, integrasjonstest eller deploy er utført, og infra-tools-kompatibilitet er ikke bevist gjennom kjøring.

| Senere verifikasjon | Eier og forventet evidens |
| --- | --- |
| Ren checkout og gjenstart med eksisterende DB-volum | #3416: repeterbar oppstart; innhold bevares og ventende migreringer utføres |
| Image-kontrakt | #3416: serve/8000, migrate exit-status, ikke-root, read-only filesystem og frontend/API-ruting |
| DB nede, deretter tilbake | #3416: live 200, ready 503 → 200; ingen restartsløyfe eller bekreftet datatap |
| To migreringsjobber og fremprovosert feil | #3416/#3424: serialisering, sjekksumkontroll og stanset utrulling ved feil |
| Gammel og ny app under release/rollback | #3424: støttet schema, tilgjengelige assets og ingen automatisk nedmigrering |
| SIGTERM midt i save | #3416/#3419: full commit eller rollback, trygg retry og bevart sist bekreftede arbeidskopi |
| OTLP borte og følsomme testverdier | #3416: avgrenset eksport, fortsatt HTTP og ingen hemmeligheter i logger/traces |
| Samme origin og API-typesjekk | #3422/#3415: frontendbygg mot kontrakt, Bokmål-ruter og korrekt 404 |
| Eksakte infra-tools-grensesnitt | #3424: kildecommit, env-mapping, collector/protokoll, migreringsbarriere og probe-manifester |

Reviewbeslutning for Erik: godkjenn eller korriger ADR-001 som implementasjonsgrunnlag, særlig én image, repostruktur og separat migreringsjobb. Infra-/sikkerhets-/kvalitetsgjennomgang bør kontrollere miljømapping, rettigheter, utrullingsrekkefølge og feilscenarioene før implementasjonen ferdigstilles. Åpne leverandør-, auth-, frontend- og domenevalg blir liggende på sine egne kort og er ikke valgt gjennom godkjenning av denne kontrakten.
