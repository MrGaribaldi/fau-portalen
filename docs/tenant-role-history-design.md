# FAU: tenantmodell, roller og historikktransaksjoner

Dato: 7. september 2026. Eier: 🧱 Systemarkitektur og datamodell. Favro: #3412.
Status: godkjent som arkitekturgrunnlag av Erik 7. september 2026 i chat («ellers synes jeg #3412 er grei»), etter avklaringen om sesjonsbaserte fullversjoner. Ikke implementert. E-postbytte følges separat i #3437; samme konto-ID skal bevares.

## Grunnlag og avgrensning

Kilder: `docs/planning-decisions.md` (særlig «Confirmed simplifications», «Latest clarifications», «MVP boundary and decision flexibility» og «Direct S3 uploads») og `prosjektgrunnlag.md`, del 3, 4, 6–8, 12 og 15. Siste intervjusvar overstyrer opprinnelig prosjektgrunnlag. MVP-grunnlaget i #3405 ble godkjent av Erik 7. september 2026 kl. 15:08:37 UTC. Dette godkjenner ikke automatisk arkitekturforslagene i dette dokumentet; senere produktendringer må fortsatt vurderes før implementasjon.

Bekreftet: én konto kan tilhøre flere FAU-er; bare administratorer inviterer og tildeler roller; alle ordinære medlemmer kan lese/redigere alle dokumenter og lese FAU-ets revisjonslogg. Tilgang krever verifisert epost og minst én gyldig rolle. Utgående administrator har seks måneders begrenset overlevering for å invitere erstattere, tildele deres roller og gi administratortilgang. Initial skoleorganisasjon og tidsroller inngår; senere skoleårsoverganger, KFAU-funksjoner, vikar og medlemsinitierte invitasjoner er utsatt.

Tabeller, intervallgrenser, transaksjonsmekanismer og begrensninger nedenfor ble utarbeidet som arkitekturforslag og er nå godkjent som arbeidsgrunnlag gjennom review av #3412. Formuleringer merket «forslag» viser opprinnelsen og er ikke ordrette intervjusvar. Eksplisitt utsatte funksjoner og åpne spørsmål på andre kort er fortsatt utsatt eller åpne. Autentiseringsleverandør og administrativ tofaktor avklares i #3414; oppbevaring/sletting i #3426; S3-isolasjon i #3435.

## Entiteter og tenantgrenser

Bruk ugjennomsiktige ID-er. ID-er er aldri tilgangsbevis. Konto og autentisering er globale; alt virksomhetsinnhold har obligatorisk `tenant_id`. Alle referanser mellom tenantdata bruker sammensatt nøkkel `(tenant_id, id)` slik at referanser på tvers av FAU-er avvises av databasen.

| Entitet | Viktige felt og integritetsregler |
| --- | --- |
| account | id, verifisert epost, verified_at, disabled_at. Ingen FAU-rolle på kontoen. |
| tenant | id, FAU-navn, status pending/active/closed, school_id. Ingen KFAU-hierarki i MVP. |
| membership | tenant_id, id, account_id, revoked_at. Unik konto per tenant. Medlemsraden alene gir ingen tilgang. |
| school | tenant_id, id, navn, kommunenavn, kommune-nettsidens skolelenke. Lenken er identifikasjonsgrunnlag, ikke verifisert rektorautorisasjon. |
| school_year | tenant_id, id, navn, starts_on, ends_on_exclusive. Bare initial konfigurasjon i MVP. |
| cohort | tenant_id, id, stabilt kullnavn/identitet. Ingen elev- eller barneregister. |
| organization_unit | tenant_id, id, school_year_id, type (trinn/klasse/base/gruppe), navn. Historiske rader bevares. |
| unit_cohort | tenant_id, unit_id, cohort_id, grade_level. Mange-til-mange støtter baser og sammenslåtte trinn. |
| unit_relation | tenant_id, parent_unit_id, child_unit_id. Samme skoleår, ingen selvreferanser eller sykluser. |
| role | tenant_id, id, navn, capability_class member/admin, valgfri unit_id/cohort_id. Rettighet følger eksplisitt klasse, aldri fritekstnavn. |
| role_assignment | tenant_id, id, membership_id, role_id, starts_on, ends_on_exclusive, revoked_at, granted_by. Slutt er obligatorisk. |
| invitation | tenant_id, id, mottakerepost, token_hash, expires_at, accepted_at, revoked_at, issuer_id, mode normal/handover. Engangsaksept. |
| invitation_role | tenant_id, invitation_id, role_id, starts_on, ends_on_exclusive. Planlagt tildeling; aktiveres først etter verifisert aksept. |
| handover_grant | tenant_id, id, source_assignment_id, starts_on, ends_on_exclusive, revoked_at. Unik kilde; skapes ved planlagt adminutløp. |
| document | tenant_id, id, title, markdown, revision, valgfri ansvarlig role_id/unit_id/school_year_id, deleted_at. Eierrolle begrenser ikke MVP-tilgang. |
| document_change | tenant_id, document_id, version_number, base_version_number, operation, actor_membership_id, occurred_at, request_id, snapshot, restored_from_version. Uforanderlig fullversjon; unik dokument/versjon. |
| edit_segment | tenant_id, document_id, id, editor_session_id, lease_generation, actor_membership_id, base_version_number, first_revision, last_revision, started_at, last_saved_at, status open/finalized, finalized_version_number. Maks ett åpent segment per dokument. Innholdet i åpent segment er document sin varig lagrede arbeidskopi. |
| edit_lease | tenant_id, document_id, holder_membership_id, editor_session_id, generation, last_change_at, expires_at. Maks én aktiv editor per dokument. |
| audit_event | tenant_id, id, actor_id, action, resource_type/id, revision, occurred_at, request_id, avgrenset metadata. Ingen token, dokumenttekst eller rå epostpayload. |
| command_receipt | tenant_id, actor_id, request_id, payload_digest, resultat-ID/revisjon. Unik per aktør og request. |
| outbox | tenant_id, id, hendelsestype, ressursreferanse, leveringstilstand. For epost og objektarbeid etter databasecommit. |

Serveren avleder aktøren fra sesjonen og sjekker valgt tenant ved hver forespørsel, inkludert søk, audit, jobber og nedlasting. Organisasjonsvelgeren viser kun tenantene kontoen faktisk har ordinær tilgang eller overleveringstilgang til. Bytte av FAU gir ingen rettighetsoverføring. Tenant-ID i URL/body valideres mot ressursen.

Forslag: PostgreSQL row-level security som ekstra lag, med applikasjonsrolle uten eier-/bypassrettigheter og tenantkontekst avgrenset til transaksjonen. Migreringer bruker separat rolle. Detaljert SQL og demonstrasjon av isolasjon hører til implementasjonen; denne spesifikasjonen beviser ikke isolasjon.

## Datoer og autorisasjon

Forslag: rolleperioder er lokale kalenderdatoer i `Europe/Oslo`, representert som halvåpne intervaller `[starts_on, ends_on_exclusive)`. UI viser siste inkluderende dato. Alle hendelsestidspunkter lagres som UTC-tidspunkter. Serverens klokke avgjør tilgang, aldri nettleserens. Krev start før slutt og avvis ugyldige perioder. Planlagt fremtidig rolle gir ingen tilgang før start.

Eksempel: UI «1. august 2026–31. juli 2027» lagres `[2026-08-01, 2027-08-01)`. Tilgangen opphører ved lokal midnatt 1. august, også dersom en bakgrunnsjobb ikke har kjørt. En annen medlemsrolle til 31. desember 2027 gir fortsatt dokument-/audittilgang, men ikke administrasjon etter adminrollens utløp.

Forslag: seks kalendermåneder regnes fra eksklusiv sluttdato med avkorting til siste gyldige dag i målmåneden. Adminutløp 1. august 2027 gir overlevering `[2027-08-01, 2028-02-01)`. Utløp 31. august gir grense 29. februar 2028. Beregn og lagre grensen eksplisitt, så samme regel brukes overalt.

En deaktivert konto, tilbakekalt medlemskap eller tilbakekalt rolle gir ikke tilgang via den tilbakekalte retten. Forslag: manuell tilbakekalling skaper ingen overlevering og tilbakekaller eventuell overlevering fra samme rolle; naturlig utløp gjør det. Overlappende gyldige roller er tillatt, og rettigheter er unionen av gyldige roller. Ingen implisitt forlengelse over sommeren.

| Operasjon | Gyldig medlem | Gyldig admin | Bare overlevering |
| --- | --- | --- | --- |
| Lese/redigere alle FAU-dokumenter og lese audit | Ja | Ja | Nei |
| Invitere og tildele tidsroller/admin | Nei | Ja | Bare egne erstatningsinvitasjoner og deres mottakere |
| Initial skolekonfigurasjon | Nei | Ja | Nei |
| Tvangsfrigjøre dokumentlås | Nei | Ja | Nei |
| Se status for egne overleveringsinvitasjoner | Nei | Ja | Ja, minimalt mottaker-/rollegrunnlag |

Forslag til konkret overleveringsbegrensning: utgående admin kan opprette, trekke tilbake og følge egne invitasjoner, med eksplisitt kobling til handover_grant, samt tildele mottakeren nye tidsroller/admin. Ingen forlengelse av egen rolle, egen invitasjon, generell medlemsoversikt, tilbakekalling av andres roller eller redigering av eksisterende organisasjon. Ny admin får ordinær tilgang først ved verifisert aksept. Disse grensene må godkjennes ved review; produktgrunnlaget angir operasjonene, men ikke alle misbruksgrenser.

Rettigheter vurderes ved hver operasjon, ikke bare ved innlogging. Forslag for mutasjoner: lås aktørens medlemsrad under autorisasjon; rolleendringer og tilbakekalling låser samme rad. Konto-deaktivering følger tilsvarende felles låseprotokoll. Bruk konsistent låserekkefølge. Revalider tidsgrenser ved skriveoperasjonen. En tilbakekalling som fullføres først skal hindre etterfølgende lagring, også med eksisterende editorlås.

## Opprettelse og initial organisasjon

Opprett tenant i pending med FAU-navn, skole, kommune, kommunens skolelenke, registrant og lederadresse. Registranten får ingen datarettigheter før egen epost er verifisert. Verifisering aktiverer FAU og gir registranten en eksplisitt tidsavgrenset adminrolle; lederens separate invitasjon gir admin først etter lederens egen verifisering. Samme verifiserte konto som registrant og leder behandles uten duplisert medlemskap. Verifisering av registranten verifiserer aldri lederadressen.

Forslag: krev sluttdato for initial adminperiode i opprettelsesflyten; ingen ubestemt bootstrap-admin. #3413 skal avklare plassering og veiledning i flyten. Aktivering, initial rolle, audit og outbox for varsel til `fau@ewb-solutions.as` lagres samlet. Epostfeil prøves på nytt uten å opprette FAU på nytt.

Invitasjonsaksept krever riktig verifisert mottaker, gyldig ubrukt token, aktiv tenant og fortsatt gyldig fullmakt hos utsteder (ordinær admin eller tilknyttet overlevering). Lås invitasjonen, opprett/gjenbruk medlemskap, aktiver planlagte roller og marker aksept i én transaksjon med audit. Forslag: selv-invitasjon for å forlenge egen overlevering avvises. Tilbakekalt/utløpt utsteder krever ny invitasjon fra gyldig admin. Planlagt rolle som ennå ikke har startet gir fortsatt ingen tilgang.

Eksempel på initial skole: skoleår 2026/27; kull K2018 og K2019; base «Blå» koblet til begge kull med henholdsvis trinn 3 og 2; klasse «3A» koblet til K2018. En rolle kan knyttes til basen eller kullet. Dette representerer struktur, ikke elever. Initial oppsett kan bygges som utkast og valideres samlet. Etter aktivering inngår ikke flytting, splitting, sammenslåing eller automatiske årsoverganger i MVP.

Historiske dokumentendringer beholder både stabile referanser og et minimalt snapshot av relevante rollenavn, enhetsnavn og skoleår. Senere omdøping skal ikke endre fortidens fremstilling. Sletting og oppbevaring av personreferanser avklares i #3426; bevarte historikkrader betyr ikke ubegrenset personoppbevaring.

## Dokument, endringssett og audit i én transaksjon

Bekreftet av Erik 7. september 2026: historiske versjoner lagres som fulle snapshots, og endringer innen én redigeringsøkt samles. 500 autosaves skal ikke lage 500 fulle historiske versjoner. «Lagre ny versjon» er foreslått som en valgfri brukerhandling som deler versjonshistorikken mens redigeringsøkten fortsetter; MVP-plassering er ikke endelig valgt.

Forslag: skill teknisk `revision` (øker ved hver faktisk innholdsendring og brukes til konfliktkontroll) fra historisk `version_number` (øker når en fullversjon ferdigstilles). Autosave lagrer Markdown og metadata varig i document og oppdaterer ett åpent edit_segment. Tidligere ferdigstilte snapshots endres aldri. Et åpent segment er arbeidsinnhold, ikke en ferdigstilt historisk versjon. Dermed bevares sist bekreftede autosave også ved krasj, uten å lagre hver mellomtilstand som fulltekst. Diff mellom ferdige versjoner kan beregnes senere.

En redigeringsøkt er knyttet til ett dokument, én aktør, editor_session_id og lease_generation, ikke hele innloggingen. Forslag til avslutningsregler:

- Normal lukking sender først siste autosave, venter på bekreftelse og ferdigstiller segmentet ved frigivelse. Hvis nettleseren forsvinner før dette lykkes, gjelder timeout; usendte klientendringer kan ikke sikres.
- 15 minutter uten dokumentendringer, tvangsfrigivelse eller tap av skrivetilgang avslutter segmentet med sist serverlagrede innhold. Serveren kan ferdigstille allerede godkjente data uten å gi den utløpte aktøren nye rettigheter. Audit skiller opprinnelig redaktør fra system/admin som avslutter.
- En jobb ferdigstiller utløpte segmenter etter krasj/frakobling; før en ny lease gis, ferdigstilles alltid et eventuelt gammelt segment under samme dokumentlås. To aktørers arbeid slås aldri sammen.
- «Lagre ny versjon», hvis innført, autosaver først og ferdigstiller segmentet atomisk. Neste endring starter nytt segment med samme lease og editorøkt. Ingen tomme versjoner ved gjentatte klikk eller åpning/lukking uten endringer. Et segment som ender identisk med basisversjonen trenger heller ingen ny fullversjon; avslutningen og utførte saves er fortsatt sporbare.

En lagringskommando har document_id, expected_revision, segment_id (eller eksplisitt start av segment), lease_generation, editor_session_id, request_id og nytt innhold. Aktør og tenant kontrolleres på serveren. Foreslått forløp:

1. Start transaksjon, lås/revalider autorisasjon og slå opp command_receipt. Samme request_id og payload gir tidligere resultat; ulik payload avvises. Tilbakekalt aktør får ikke hente tidligere innhold via retry.
2. Lås dokument og lease. Verifiser tenant, at dokumentet ikke er slettet, skrivetilgang, lease-holder/session, generation, utløp, expected_revision og at segmentet fortsatt er åpent. En forsinket forespørsel mot et ferdigstilt segment avvises, også når lease beholdes etter manuell versjonsdeling.
3. Ved faktisk endring: øk teknisk revision, oppdater document og edit_segment, legg til en liten audit_event med aktør, tidspunkt, segment og revision, oppdater lease og lagre command_receipt. Ingen fulltekst i audit eller receipt. Uendret autosave lager ingen ny versjon og forlenger ikke inaktivitet.
4. Commit før suksess returneres. Feil i segment/audit/receipt ruller hele autosave tilbake. Samtidige requests serialiseres med dokumentlås, unik receipt-nøkkel og transaksjonsretry.

Ferdigstilling bruker samme dokumentlås: les sist lagrede arbeidskopi, opprett én uforanderlig document_change med fullt snapshot, merk segmentet avsluttet, skriv audit og receipt i samme transaksjon. Unik kobling segment/ferdigstilt versjon hindrer dobbeltversjon ved retry, samtidig timeout og manuell lagring. Klienten serialiserer autosave og manuell ferdigstilling; serveren avviser stale revision/segment uansett klientadferd.

Opprettelse og .txt/.md-import lager ferdig versjon 1 med audit atomisk. Etter dette leser ordinære dokumentvisninger siste varig lagrede arbeidskopi, mens fremtidig historikkvisning viser ferdige versjoner. Restore må ferdigstille et eventuelt åpent segment før gjenopprettingen, i samme atomiske operasjon, slik at allerede lagret arbeid ikke forsvinner.

Eksempel: dokumentet har ferdig versjon 3. En bruker skriver 500 tegn med mange autosaves. Arbeidskopiens tekniske revision øker, mens én samlet versjon 4 opprettes ved avslutning. Trykker brukeren «Lagre ny versjon» halvveis, blir første del versjon 4 og senere endringer versjon 5 ved avslutning. De enkelte autosavene har små auditposter, men ingen fulle snapshots; mellomliggende teksttilstander kan derfor ikke gjenopprettes. Dette er den tilsiktede lagringsavveiningen. Mål bytes per tenant og samordne oppbevaring av snapshots, audit og receipts med #3426.

Audit er applikasjonsdata, ikke bare driftslogger. Applikasjonsrettigheter skal hindre UPDATE/DELETE på ferdigstilt document_change og audit; den aktive arbeidskopien og segmentmetadata må kunne oppdateres. Privilegert vedlikehold/sletting krever egen kontrollert prosedyre. Dette beviser ikke beskyttelse mot en privilegert databaseadministrator; videre arbeid ligger i #3421.

Lås tas også for samme bruker i to faner: editor_session_id identifiserer editoren. Første lease får 15 minutter fra opprettelse; deretter regnes inaktivitet fra siste faktiske dokumentendring. Heartbeat forlenger ikke lease. Lukking forsøker eksplisitt frigivelse; ved mistet forbindelse gjelder timeout. Tvangsfrigivelse låser dokument/lease, øker generation og logger handlingen atomisk. Sist serverlagrede innhold beholdes. Alle sene saves med gammel generation avvises, selv om ny lease senere tilhører samme konto.

Eksempel: revisjon 7/generation 12 er lagret. Admin frigjør låsen til generation 13. Gammel editor sender revisjon 8 med generation 12: avvis, ikke lagre eller opprette change. Dersom lagringen fullførte før frigivelsen, er revisjon 8 den siste serverlagrede versjonen som beholdes. Vis avvisningen tydelig; usendte klientendringer er ikke gjenopprettet.

Audit leses tenantfiltrert av gyldige medlemmer. Bare overlevering gir ikke tilgang. Metadata skal beskrive handling og ressurs uten å lekke invitasjonstoken, hemmeligheter eller full dokumenttekst.

## S3 og fremtidig gjenoppretting

PostgreSQL og S3 behandles som separate transaksjonsdomener. Dersom originalfil beholdes: opprett pending metadata med servergenerert nøkkel; autoriser kortlevd direkte opplasting; kontroller objektets størrelse/innhold på backend før finalisering. Først ved finalisering kobles validert objekt til dokumentets første versjon i databasetransaksjonen. Et objekt som eksisterer alene er ikke tilgjengelig innhold. Outbox/jobb rydder foreldreløse objekter idempotent. Replay/overskriving og objektets stabilitet mellom validering og bruk må løses i #3435 før dette implementeres; bøttetopologi er fortsatt åpen.

Fremtidig dokumentrestore leser et ferdigstilt snapshot og oppretter en NY historisk versjon med operation=restore og restored_from_version; teknisk revision økes også. Historiske versjoner overskrives aldri. Normal autorisasjon, lease, expected_revision og audit gjelder. Eksempel uten åpent segment: restore av versjon 3 når siste ferdige er 9 lager versjon 10; 4–9 bevares. Finnes et endret åpent segment, ferdigstilles det som 10 og restore blir 11, atomisk. Brukergrensesnitt og endpoint for restore er utsatt.

Dokumentrestore er forskjellig fra katastrofegjenoppretting av database/objektlager. Sistnevnte skal verifiseres i #3425 og krever samordnet innhold, referanser og historikk. Ingen backupgaranti gis her.

## Akseptansegjennomgang og videre verifikasjon

Dette er en dokumentgjennomgang, ikke kjørte programvaretester. Følgende scenarioer er krav til senere integrasjonstester i #3428 og implementasjonskortene:

| Scenario | Forventet resultat |
| --- | --- |
| Konto medlem i A og B bytter FAU; sender A-dokument-ID under B | Avvis uten innholdslekkasje; sammensatt FK avviser kryssreferanse |
| Medlem uten admin inviterer eller gir rolle | Avvis |
| Rolle starter i morgen, utløper ved midnatt eller tilbakekalles | Ingen tilgang uten annen gyldig rolle; grensene testes rundt lokal midnatt og sommertid |
| Admin utløper, annen medlemsrolle består | Dokument/audit tillatt; administrasjon bare innen overleveringsoperasjonene |
| Bare overlevering, eller seks måneder passert | Bare avgrenset handover før grensen; ingen tilgang etter |
| Registrant verifisert, leder uverifisert; eller de er samme konto | Bare verifisert konto får tilgang; ingen duplikat |
| Invitasjon aksepteres to ganger eller etter tilbakekalling av utsteder | Ingen duplisert rolle; ugyldig aksept avvises |
| Base omfatter to kull; dokument refererer initial struktur | Gyldig representasjon uten elevdata eller automatisk overgang |
| Audit-/historikkskriving feiler under autosave | Ingen delvis oppdatert dokumentrevisjon |
| Samtidig save, retry etter mistet svar, samme ID med ulik payload | Én teknisk revisjon per faktisk endring; ingen ekstra historiske fullversjoner ved retry |
| 500 autosaves i én økt; manuell deling halvveis | Én ny fullversjon ved avslutning; to dersom brukeren deler og fortsetter å endre |
| Krasj, timeout og ny aktør; samtidig manuell ferdigstilling | Sist bekreftede innhold bevares; segment ferdigstilles nøyaktig én gang før ny aktør slipper til |
| Forsinket autosave etter manuell deling; tomt eller uendret segment | Ferdigstilt segment kan ikke endres; ingen tom historisk versjon |
| Rolle tilbakekalles samtidig med save | Felles låseprotokoll gir deterministisk rekkefølge; save etter tilbakekalling avvises |
| Tvungen frigivelse, timeout, gammel fane eller uendret heartbeat | Gammel generation avvises; ingen utilsiktet forlengelse |
| Fremtidig restore 3 → 10 og endret organisasjonsnavn | Mellomliggende revisjoner og datidens metadata bevares |
| Objekt finnes i S3 men metadata er pending | Ingen nedlasting eller synlig import før validert finalisering |

Alle seks akseptanseområder i #3412 er behandlet: konto i flere FAU-er, datosemantikk, initial skoleorganisasjon, seks måneders begrenset overlevering, atomisitet og fremtidig restore. Review av #3412 er godkjent, inkludert datogrenser, bootstrap-periode, overleveringsbegrensninger og avslutningsregler som arkitekturgrunnlag. Fulle snapshots og samling per økt er bekreftet. «Lagre ny versjon» er fortsatt en valgfri funksjon; godkjenningen endrer ikke eksplisitt utsatt MVP-omfang. Innarbeid eventuelle korreksjoner fra #3405. #3413 konkretiserer medlemsflyten; #3416/#3418/#3419/#3421 bruker modellen etter review.
