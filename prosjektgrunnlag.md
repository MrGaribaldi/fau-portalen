# FAU-plattform – prosjektgrunnlag

**Oppdatert:** 7. september 2026  
**Formål:** Samlet kontekst for videre produktutvikling, arkitekturarbeid og planlegging i Claude.

## Instruks til Claude

Bruk dette dokumentet som prosjektets gjeldende utgangspunkt. Løsningen og premissene nedenfor er valgt retning. Hjelp med å konkretisere krav, datamodell, brukerflyt, sikkerhet, oppgaver og implementasjon. Dersom nye forslag bryter med premissene, skal konsekvensene forklares tydelig.

## 1. Produktet

Vi skal bygge en webapplikasjon for norske FAU-er. Plattformen skal samle dokumenter, kunnskap, møtereferater, oppgaver, organisasjon og historikk på ett sted.

Løsningen skal:

- bevare kunnskap og dokumentasjon når personer og verv skiftes ut;
- gi medlemmene egne brukerkontoer og riktig tilgang;
- gjøre det mulig å opprette, importere, redigere og versjonere dokumenter i nettleseren;
- vise hvem som opprettet, endret, flyttet, publiserte eller slettet innhold, og når det skjedde;
- organisere ansvar rundt roller fremfor å gjøre innholdet avhengig av bestemte personer;
- håndtere skoler med ulike og skiftende trinn-, klasse-, base- og gruppestrukturer;
- støtte wiki-lignende kunnskapsinnhold og enkel oppgaveoppfølging;
- publisere godkjente offentlige referater som HTML og PDF;
- være enkel å bruke for frivillige foreldre uten teknisk kompetanse;
- være GDPR-kompatibel og ha personvernvennlige standardinnstillinger.

## 2. Pris og lagring

Tjenesten vil trolig koste **2 400 kroner pluss merverdiavgift per FAU per år**.

Kundene skal ikke få oppgitt en bestemt lagringskvote i markedsføringen eller abonnementet. Tjenesten selges som en helhetlig løsning, ikke som et antall gigabyte.

Infrastrukturen dimensjoneres likevel slik at hvert FAU i praksis kan bruke opptil **yyy GB** data. `yyy` er en intern kapasitetsgrense som skal fastsettes ut fra kostnader, forventet normalbruk og behovet for å beskytte tjenesten mot ekstrembruk. Grensen trenger ikke vises for kunden så lenge bruken er normal.

Lagringsmodellen inkluderer aktive dokumenter, opplastede originalfiler, bilder, PDF-er, dokumentversjoner, revisjonshistorikk og sikkerhetskopier. Systemet skal måle lagringsbruk og kostnad per FAU.

## 3. Teknisk plattform

Tjenesten skal kjøres hos **Hetzner**.

Infrastrukturen etableres med **Terraform**, som setter opp et **Kubernetes-cluster**. Plattformen skal også omfatte PostgreSQL, S3-kompatibel objektlagring, logging, overvåkning, sikkerhetskopiering og nødvendige driftskomponenter.

Hovedkomponentene er:

- webapplikasjon som brukergrensesnitt;
- backend/API for forretningslogikk og tilgangskontroll;
- PostgreSQL for strukturert data, redigerbart dokumentinnhold, metadata og versjonshistorikk;
- Hetzners S3-kompatible objektlagring for opplastede originalfiler og binærfiler;
- Kubernetes for kjøring og skalering av tjenestene;
- Terraform for reproduserbar infrastruktur;
- sentralisert logging, overvåkning og varsling.

Løsningen bygges som en flerleietakertjeneste. Hvert FAU er en egen tenant, og data skal være strengt separert mellom organisasjonene. All autorisasjon håndheves på serversiden.

## 4. Dokumentmodell og lagring

### 4.1 Redigerbart innhold

Dokumenter som redigeres i webapplikasjonen, lagres som **Markdown i PostgreSQL**. Innholdet lagres sammen med strukturert metadata og versjonshistorikk.

For hvert dokument lagrer systemet blant annet:

- tittel og dokumenttype;
- Markdown-innhold;
- eier eller ansvarlig rolle;
- tilhørende FAU, skoleår og organisatorisk enhet;
- tilgangsnivå;
- opprettet av og opprettet tidspunkt;
- sist endret av og endringstidspunkt;
- komplett versjonshistorikk;
- publiseringsstatus;
- referanse til opplastet originalfil når dokumentet er importert.

Markdown gir en enkel og robust redigeringsmodell, gjør endringer lette å sammenligne og gjør innholdet egnet for søk, gjenbruk og eksport.

### 4.2 Originalfiler og binærfiler

Opplastede originalfiler lagres i **Hetzners S3-løsning**. Det samme gjelder filer som ikke naturlig lagres som redigerbar tekst, blant annet PDF-er, bilder, opprinnelige DOC-/DOCX-filer, opprinnelige XLS-/XLSX-filer og andre vedlegg.

Når et dokument importeres, beholdes originalfilen i objektlageret. Det konverterte, redigerbare innholdet lagres som Markdown eller annen strukturert data i PostgreSQL. Relasjonen mellom originalen og det redigerbare dokumentet skal bevares.

Objekter i S3 skal ha metadata i PostgreSQL. Databasen er autoritativ for tilgang, eierskap, koblinger, status og historikk; brukerne skal aldri gis ukontrollert direkte tilgang til objektlageret.

## 5. Import av dokumenter

Alle vanlige dokumenttyper skal kunne lastes opp og importeres. Dette omfatter blant annet:

- DOC og DOCX;
- Markdown (`.md`);
- XLS og XLSX;
- CSV og TSV;
- ODT og andre relevante åpne dokumentformater;
- ren tekst;
- PDF;
- bilder.

Importflyten skal:

1. lagre originalfilen uendret i Hetzners S3-løsning;
2. kontrollere filtype, størrelse og sikkerhet;
3. hente ut innhold og metadata;
4. konvertere tekstlig innhold til Markdown eller annen egnet struktur;
5. opprette et redigerbart dokument i webapplikasjonen;
6. knytte originalfilen til den importerte versjonen;
7. vise brukeren en forhåndsvisning før importen godkjennes;
8. registrere importen i revisjonsloggen.

Importen skal bevare mest mulig av overskrifter, avsnitt, lister, lenker, enkle tabeller, utheving og relevante metadata. Kompleks layout trenger ikke gjenskapes nøyaktig; innhold og struktur prioriteres foran pikselidentisk formatering.

### 5.1 PDF og bilder

Når tekst kan hentes fra en PDF, skal brukeren få tilbud om å importere teksten som et redigerbart dokument. For skannede PDF-er og bilder kan systemet bruke OCR.

Brukeren skal få se resultatet før det lagres som redigerbart innhold. Originalen beholdes alltid, slik at feil i tekstuttrekk eller OCR kan kontrolleres mot kilden.

### 5.2 Regneark

Regneark importeres som strukturert regnearkinnhold og skal kunne vises og redigeres i nettleseren. Løsningen skal støtte typiske FAU-behov som:

- enkle budsjetter;
- inntekts- og utgiftsoversikter;
- deltaker- og oppgavelister;
- tabeller;
- grunnleggende formler;
- import og eksport av XLS, XLSX og CSV.

Regnearkfunksjonen skal bygges med et etablert og godt testet bibliotek. Vi skal ikke utvikle en egen regnearkmotor.

## 6. Dokumentredigering og versjonering

Brukerne redigerer dokumenter i en enkel nettleserbasert editor med støtte for Markdown-funksjoner gjennom et brukervennlig grensesnitt.

Editoren skal støtte overskrifter, avsnitt, lister, sjekklister, lenker, enkel formatering, tabeller, vedlegg, bilder, intern lenking, forhåndsvisning og eksport til PDF.

Hver lagrede endring oppretter en sporbar versjon. Brukeren skal kunne:

- se versjonshistorikken;
- se hvem som gjorde en endring og når;
- sammenligne to versjoner;
- åpne en eldre versjon;
- gjenopprette en eldre versjon uten at mellomliggende historikk forsvinner.

Første versjon trenger ikke samtidig sanntidsredigering som i Google Docs. Datamodellen bør ikke blokkere at dette kan innføres senere.

## 7. Roller, medlemmer og tilgang

Systemet organiseres rundt roller. Personer innehar roller i en angitt periode, mens ansvar, dokumenter, oppgaver og historikk kan fortsette å tilhøre rollen.

Eksempler på roller er FAU-leder, nestleder, kasserer, sekretær, styremedlem, trinnrepresentant, klasse-/base-/grupperepresentant og vara.

Tilganger skal kunne gis ut fra FAU-medlemskap, rolle, organisatorisk enhet, dokumentområde, internt eller offentlig innhold og administrativt ansvar.

Roller og medlemskap skal ha start- og sluttdato. Når en person går ut av et verv, skal den nye personen kunne overta relevant ansvar uten å overta den forrige personens identitet eller private konto.

## 8. Fleksibel skoleorganisasjon

Systemet må støtte at skoler organiserer elevkull ulikt og at organiseringen kan endres mellom skoleår.

Det skal kunne representere:

- trinn, for eksempel «3. trinn»;
- klasser, for eksempel «3A» og «3B»;
- grupper, for eksempel «gul gruppe på 2. trinn»;
- baser, for eksempel «base 5»;
- sammenslåtte trinn;
- skoler uten faste klasser;
- kombinasjoner der enkelte trinn har klasser og andre bare er ett samlet trinn.

Datamodellen skal skille mellom skole, skoleår, elevkull, trinnnivå, organisatorisk enhet, relasjoner mellom enheter, roller, vervsperioder og personer som innehar rollene.

Et elevkull flyttes normalt opp ett trinn hvert skoleår, mens den organisatoriske inndelingen kan bestå, deles eller slås sammen. Systemet skal bevare sammenhengen i historikken gjennom slike endringer.

For representanter som velges for to år, skal rollen kunne følge elevkullet automatisk til neste trinn. Automatikken må bruke elevkull og vervsperiode, ikke bare teksten i et rollenavn.

Historiske dokumenter, møter, roller og beslutninger skal vises med organisasjonen og navnene som gjaldt da innholdet ble opprettet.

## 9. Møter og referater

FAU-et skal kunne opprette møteinnkallinger, sakslister og referater i webapplikasjonen.

Et referat skal kunne inneholde møtedato og sted, deltakere og fravær, saksliste, beslutninger, oppgaver, ansvarlig rolle, frist, interne notater, offentlig tekst, vedlegg og godkjennings-/publiseringsstatus.

Internt og offentlig innhold må være tydelig atskilt. Bare godkjent offentlig innhold publiseres. Offentlige referater publiseres som tilgjengelig HTML og generert PDF.

Publiserte sider skal som standard bruke `noindex` og andre relevante HTML- og HTTP-signaler for å motvirke indeksering fra søkemotorer og AI-tjenester. Et FAU kan uttrykkelig velge at bestemte offentlige sider skal kunne indekseres.

Systemet skal lagre hvilken versjon som ble publisert, hvem som godkjente og publiserte den, og tidspunktet for publisering. Senere endringer oppretter en ny versjon og overskriver ikke publiseringshistorikken.

## 10. Wiki og kunnskapsbase

Markdown-dokumentene skal også kunne brukes som en wiki-lignende kunnskapsbase for årshjul, rutiner, arrangementer, sjekklister, leverandør- og kontaktinformasjon, maler, overleveringsnotater, beslutningshistorikk og veiledning for roller.

Dokumenter skal kunne lenkes til hverandre, kategoriseres og finnes gjennom søk. Kunnskap skal kunne eies av en rolle eller organisatorisk enhet, slik at den ikke blir knyttet til personen som skrev den.

## 11. Oppgaver

Plattformen skal støtte enkel oppgaveoppfølging knyttet til møter, dokumenter, arrangementer og organisatoriske enheter.

En oppgave skal kunne ha tittel, beskrivelse, ansvarlig person eller rolle, frist, status, prioritet, kobling til møte/sak/dokument, kommentarer og endringshistorikk. Når en rolle skifter innehaver, skal åpne oppgaver kunne følge rollen videre.

## 12. Revisjonslogg

Alle viktige handlinger skal logges. Dette inkluderer opprettelse og endring av dokumenter, import og eksport, opplasting og sletting av filer, gjenoppretting av versjoner, endringer i roller og tilganger, publisering av referater og administrative endringer.

Revisjonsloggen skal vise hvem som gjorde hva og når. Hendelser skal knyttes til riktig FAU og beskyttes mot endring. Tilgangen til revisjonsloggen skal være rollebasert.

## 13. GDPR og sikkerhet

Løsningen skal bygges med personvern og sikkerhet som standard. Dette omfatter:

- databehandleravtale og dokumentert rollefordeling;
- kryptering under overføring og lagring;
- rollebasert tilgang og minste privilegium;
- tofaktorautentisering for administrative roller;
- sikker invitasjons- og kontogjenopprettingsflyt;
- virusskanning og validering av opplastede filer;
- revisjonslogg;
- regler for oppbevaring og sletting;
- eksport og dataportabilitet;
- fullstendig sletting ved avsluttet kundeforhold;
- sikkerhetskopiering og testet gjenoppretting;
- håndtering og varsling av sikkerhetshendelser;
- streng dataseparasjon mellom FAU-er.

Offentlig publisering skal ha en egen kontrollflyt som reduserer risikoen for at personopplysninger eller interne vedlegg publiseres ved en feil.

## 14. Eksport og eierskap til data

FAU-et eier sitt innhold og skal kunne eksportere det i brukbare formater. Eksporten skal omfatte Markdown-dokumenter, PDF-versjoner, originalfiler, regneark, organisasjonsstruktur, tillatt medlems- og rollehistorikk, oppgaver, publiseringshistorikk og relevante metadata.

Eksporten skal være strukturert slik at et FAU kan arkivere innholdet eller flytte det til en annen løsning.

## 15. Tekniske prinsipper

- Webapplikasjonen er det primære arbeidsverktøyet.
- Redigerbart tekstinnhold lagres som Markdown i PostgreSQL.
- Opplastede originalfiler og binærfiler lagres i Hetzners S3-kompatible objektlager.
- PostgreSQL er autoritativ for metadata, tilgang, koblinger og historikk.
- Infrastrukturen kjøres hos Hetzner i Kubernetes og opprettes med Terraform.
- Hvert FAU er en separat tenant.
- Autorisasjon håndheves på serversiden.
- Dokumenter og organisasjonsdata versjoneres fremfor å overskrives destruktivt.
- Modne biblioteker brukes til editor, dokumentkonvertering, OCR og regneark.
- Kritiske avhengigheter vurderes for lisens, sikkerhet, vedlikehold og støtte for selvhosting.
- Alle sentrale tjenester skal ha logging, overvåkning, sikkerhetskopi og testet gjenoppretting.

## 16. Første leveranse

Første produksjonsklare leveranse skal gjøre det mulig å:

1. opprette et FAU som egen organisasjon;
2. invitere medlemmer og tildele tidsavgrensede roller;
3. konfigurere skoleår, elevkull, trinn, klasser, baser og grupper;
4. opprette og redigere Markdown-baserte dokumenter;
5. importere vanlige dokument- og regnearkformater;
6. tilby tekstuttrekk eller OCR for PDF og bilder der det er mulig;
7. beholde opplastede originalfiler i S3;
8. vise, sammenligne og gjenopprette dokumentversjoner;
9. laste opp, vise og laste ned vedlegg;
10. opprette møter, saker og referater;
11. skille internt og offentlig referatinnhold;
12. publisere godkjente referater som HTML og PDF med indeksering avslått som standard;
13. opprette og følge opp enkle oppgaver;
14. bruke dokumentene som kunnskapsbase;
15. søke i innhold brukeren har tilgang til;
16. se revisjonslogg for viktige hendelser;
17. eksportere FAU-ets innhold;
18. håndtere sikkerhetskopi, gjenoppretting, sletting og andre grunnleggende GDPR-behov.

## 17. Prosjektorganisering

Videre AI-assistert prosjektarbeid organiseres med spesialistagenter fremfor én generell agent. Hver agent analyserer prosjektet fra sitt fagområde, stiller nødvendige spørsmål og styrer sine oppgaver.

Favro brukes til prosjektstyring:

- prosjektet får en egen collection;
- hver spesialistagent får sitt eget board;
- eksisterende Favro-skill og CLI brukes;
- agentene bruker forskjellige emojier i kommentarer, slik at bidragene er lette å skille.

Agentroller:

- 🧭 Produkt og prosjektledelse
- 👥 Brukerbehov og FAU-organisering
- 🧱 Systemarkitektur og datamodell
- 🔐 Sikkerhet og GDPR
- 💾 Infrastruktur, drift og lagringsøkonomi
- 🎨 UX og universell utforming
- 💰 Forretningsmodell, marked og prising
- ⚖️ Juridiske rammer og avtaler
- 🧪 Kvalitet, test og risiko
- 📣 Salg, pilotering og kundekontakt

## 18. Kort prosjektbeskrivelse

FAU-plattformen er en GDPR-tilpasset webapplikasjon for norske FAU-er. Den samler dokumentredigering, import, versjonering, originalfiler, organisasjon, referater, kunnskapsbase og oppgaver i én løsning. Redigerbart innhold lagres som Markdown i PostgreSQL, mens opplastede originalfiler og binærfiler lagres i Hetzners S3-kompatible objektlager. Løsningen kjøres hos Hetzner i et Kubernetes-cluster opprettet med Terraform. Organisasjonen modelleres rundt roller, elevkull og tidsavhengige skoleenheter, slik at historikk og ansvar overlever årlige utskiftninger og organisatoriske endringer. Tjenesten vil trolig koste 2 400 kroner pluss merverdiavgift per FAU per år. Kundene får ikke oppgitt en fast lagringskvote, mens infrastrukturen dimensjoneres for opptil `yyy GB` per FAU.
