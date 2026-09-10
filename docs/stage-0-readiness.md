# Steg 0: klargjøring og SSH-nøkler

Kontrollert 7. september 2026 mot infra-tools commit `6c301e15e3ee83a9797971046ffe1bc9b8593ca3`. Kun fil-/miljøinspeksjon; ingen init, plan eller apply utført.

## SSH: bruk generert nøkkel

`infrastructure/0-persistent/main.tf` genererer RSA 4096 via `tls_private_key.generated`, registrerer public key via `hcloud_ssh_key.generated`, og skriver privat/offentlig nøkkel til `infrastructure/.config/ssh_key/id_ed25519` og `.pub`. Filnavnet er misvisende: nøkkelen er RSA, ikke Ed25519. Privatnøkkelfilen får 0600. Steg 1 bruker samme genererte privatnøkkel fra persistent_outputs.json. Ingen egen manuelt generert nøkkel er nødvendig.

Dette registrerer nøkkelen i Hetzner Cloud, ikke Robot. Dedicated-serverens forberedelse krever at den genererte public key registreres/velges i Robot og at SSH-tilgang beholdes i installert OS. Bare public key skal kopieres til Robot. IP, Robot server-ID og serverreinstallasjon er senere steg; de er ikke nødvendige for å generere nøkkelen i steg 0.

## Faktisk status i dette arbeidsmiljøet

- Terraform mangler i PATH. Repoet krever >= 1.15.0 og bruker const-variabler i module source. Installer kompatibel Terraform før init; ingen validate/plan er kjørt.
- `infrastructure/.credentials.tfvars` finnes ikke. Ingen TF_VAR_-variabler er satt. Ingen credentialverdier er skrevet ut i kontrollen.
- Ingen `.terraform`, lockfil, lokal state eller `persistent_outputs.json` finnes i steg 0/forventet outputplassering. Det er ikke bevis på at ressursene aldri har vært opprettet fra en annen maskin; eventuell eksisterende state skal gjenbrukes.
- SOPS, age, WireGuard og kubectl mangler også i PATH, men er ikke CLI-avhengigheter for selve steg 0 slik koden står. Terraform-providere genererer nøklene. Disse verktøyene trengs til senere drift/steg.

## Input før uendret steg 0 kan kjøres

| Input | Krav nå |
| --- | --- |
| Hetzner Cloud-prosjekt og read/write API-token | Påkrevd for registrering av SSH-nøkkel. Oppgis i lokal ignorert credentialfil som `hcloud_token`. |
| S3 access/secret key | Påkrevd: `s3_access_key`, `s3_secret_key`. Steg 0 oppretter to private backupbøtter uten enable/disable-bryter. Lav backupprioritet gjør dem ikke automatisk valgfrie. |
| Grafana admin-passord | Variabelen `grafana_admin_password` mangler default og må fylles inn; kan genereres lokalt. Brukes senere, men kreves som input nå. |
| Prosjektnavn/lokasjon/bøttenavn/nøkkelnavn | Standard er `infra-tools`, `hel1`, `infra-tools-k3s-backup`, `infra-tools-db-backup`, SSH-navn `admin`. Foreslå `fau-mvp` og `fau-mvp-admin` med prosjektavledede bøttenavn; avklar lokasjon mot dedikert server og S3 før apply. Kontroller navnekollisjoner. |
| Robot API-bruker/passord | Kan være tomme i steg 0, men trengs senere for valgt dedicated/CCM-løype. Oppgis som `hcloud_robot_user`/`hcloud_robot_password`; ikke nødvendig for lokal nøkkelgenerering. |
| Cloudflare, GitHub dashboards-token, varsling | Har tomme defaults i steg 0 og kan vente. Gjeldende steg 2 aktiverer dashboards-sync og krever PAT; DNS/øvrige integrasjoner må avklares før relevante senere steg. |

Cloud-token og S3-nøkler opprettes ikke av Terraform-koden. De må finnes først. S3-opprettelse kan utløse kostnader og skal fremgå av den konkrete planen. Floating IP er avslått i gjeldende konfigurasjon. Steg 0 oppretter ingen VM, dedikert server, LB eller operativsystem.

## Lokal klargjøring før kjøring

Bruk FAU-konfigurasjon adskilt fra upstream-eksemplet og fest modulreferansen. Behold state og genererte nøkler mellom kjøringer; de er nødvendig varig oppsett selv når applikasjonsbackup er nedprioritert. `persistent_outputs.json` inneholder både API-hemmeligheter og privatnøkler; den bruker nå local_file uten eksplisitt 0600. Sett restriktive filrettigheter for denne og state/credentialplassering før apply. Git-ignore alene gir ikke filbeskyttelse. Ikke vis plan/state/handoff-filer ukritisk i chat.

Rekkefølge: installer Terraform → tilpass prosjektkonfigurasjon og filrettigheter → legg credentials sikkert lokalt → init → validate → plan uten interaktiv input → vurder konkrete ressursendringer → apply. Etter steg 0 brukes den genererte public key til Robot/OS-installasjonen. Ingen serverreset er nødvendig før dette.

## Oppdatert status i FAU-agentmiljøet — 8. september 2026

Kontrollert i den kjørende agent-boxen (`fau-mvp-agent-1`, `agent-box:v1.4.2`), som erstatter miljøbeskrivelsen over.

Til stede og tilstrekkelig for steg 0: terraform 1.16.1 (>= 1.15.0 kreves), docker compose v5.5.1, node 22.23.2, npm 10.9.8, python 3.13.5, kubectl 1.37.0, git 2.47.3, ssh, jq. `/workspace` er skrivbar, `/opt/infra-tools` read-only på commit `6c301e15e3ee83a9797971046ffe1bc9b8593ca3`, og `/infra-runtime/infrastructure` finnes med `.credentials.tfvars` og `favro.env` i modus 0600. Nettverk ut til registry.terraform.io, crates.io og favro.com er bekreftet.

Manglet i basisimaget: rust/cargo, **all** C-kompilator (ingen cc/gcc/make/pkg-config), favro, sops, age, wireguard-tools, psql, codex. Ingen sudo; bruker er `claude` (uid 1000, i docker-gruppen).

Steg 0 er ikke avhengig av sops-, age- eller wg-binærer: `shared-modules/sops/main.tf` lager age-nøkkelen med provideren `clementblaise/age`, og WireGuard-nøklene kommer fra `jackivanov/x25519`. VPN-verktøy trengs først fra steg 1, og NET_ADMIN/`/dev/net/tun` er bevisst ikke gitt i Compose.

Favro-CLI krever Rust >= 1.82 og var derfor den eneste faktiske blokkeringen for koordinering. Den er nå bygget inn i `Dockerfile.agent` (`fau/agent-box:v1.4.2-fau`) sammen med Rust 1.98.1, sops 3.11.0 (samme versjon infra-tools pinner, sjekksummert), age, wireguard-tools og postgresql-client. `favro 0.2.0`, `favro check` og `favro list-collections` er verifisert mot eksisterende collection FAU-plattform fra det bygde imaget. Verten må gjenopprette agent-tjenesten før den kjørende containeren får verktøyene.

## S10 utført: FAU-eid steg-0-root — 8. september 2026

Steg-0-kildefilene er kopiert fra `/opt/infra-tools/infrastructure/0-persistent` til
`/workspace/infrastructure/0-persistent`. Delte moduler i `/opt/infra-tools/shared-modules`
er ikke endret; read-only-mounten gjør det heller ikke mulig.

### Valgte verdier

Erik valgte 8. september 2026: lokasjon `hel1`, klyngenavn `fau` (Hetzner-prosjektet heter
FAU-portal; kortnavnet holdes enkelt), og eksplisitt 0600 på outputfilen.

| Verdi | infra-tools | FAU |
| --- | --- | --- |
| `cluster_name` | `infra-tools` | `fau` |
| `hcloud_location` | `hel1` | `hel1` |
| `hcloud_ssh_key_name` | `admin` | `fau-admin` |
| `s3_endpoint` / `s3_region` | `hel1.your-objectstorage.com` / `hel1` | uendret |
| `k3s_backup_bucket_name` | `infra-tools-k3s-backup` | `fau-k3s-backup` |
| `db_backup_bucket_name` | `infra-tools-db-backup` | `fau-db-backup` |
| `vpn_router_assign_floating_ip` | `false` | uendret |

`providers.tf` og `storage.tf` er byte-identiske med upstream. Alle bøttenavn og
SSH-nøkkelnavnet er nye, så ingen kollisjon med infra-tools sine ressurser.

### Modulreferanse

`infra_tools_path` er satt til `/opt/infra-tools` og `infra_tools_ref` til tom streng, altså
lokal read-only mount. Eneste modulkall er `shared-modules/sops`, og stien er kontrollert til
å eksistere. Git-alternativet er dokumentert i kommentaren i `variables.tf` med commit
`6c301e15e3ee83a9797971046ffe1bc9b8593ca3`; det krever GitHub-lesetilgang ved init.

### To bevisste avvik fra upstream

1. `local_file.persistent_output` har fått `file_permission = "0600"`. Filen inneholder
   hcloud-token, S3-nøkler, admin-privatnøkkelen, WireGuard-privatnøklene og SOPS-age-nøkkelen;
   upstream lar den ligge på `local_file` sin 0644-default.
2. `module.sops` kalles med `sops_filename = "../.sops.yaml"`, ikke `"../../.sops.yaml"`.
   **Dette retter en faktisk feil i pre-0-guiden.** Terraform kjører i
   `/infra-runtime/infrastructure/0-persistent`, så upstream-stien peker på
   `/infra-runtime/.sops.yaml`. `/infra-runtime` er `root:root 0755` og agenten kjører som
   `claude`; kontrollert med skrivetest at det gir Permission denied. Steg 0 ville feilet under
   apply. Filen legges nå i `/infra-runtime/infrastructure/.sops.yaml`. Modulen er uendret —
   avviket ligger i FAU sitt rotkall.

`.sops.yaml` inneholder bare age-**public**-nøkkelen og er den ene genererte filen som kan
kopieres tilbake til `/workspace`. Det gjøres først etter apply.

### Synkronisering til runtime

`/workspace/infrastructure/sync-to-runtime.sh` kopierer én vei, workspace → runtime, og bare de
fem filene i en eksplisitt allow-list per steg. Den sletter aldri noe, avviser beskyttede
navnemønstre (`*.tfstate*`, `*.tfvars`, `*.tfplan`, `.terraform*`, `.config`,
`persistent_outputs.json`, nøkkelfiler), oppretter målmappen 0700, skriver filer 0600, og
varsler om ufordøyde `.tf`-filer i runtime uten å røre dem. `--check` gir tørrkjøring og
exit 1 ved avvik. Kjørt: fem filer synkronisert, andre kjøring rapporterer ingen drift.

### Filrettigheter og outputplassering

`/infra-runtime/infrastructure` er 0700 `claude:claude`. `0-persistent/` er 0700, `.tf` 0600.
`.config/` og `.config/ssh_key/` er forhåndsopprettet 0700, fordi `local_file` sin
`directory_permission` ellers er 0777. Etter apply havner `persistent_outputs.json` og
`ssh_key/id_ed25519{,.pub}` der. Ingen eksisterende state ble overskrevet: det fantes ingen
`.terraform`, lockfil eller state i runtime-roten før synkroniseringen.

`terraform fmt -check -recursive` er ren. `init`, `validate` og `plan` tilhører S12 og er ikke
kjørt her.

## S12 utført: init, validate og plan — 8. september 2026

Kjørt i `/infra-runtime/infrastructure/0-persistent` med `umask 077`. Ingen apply.

### Verktøy- og providerversjoner

Terraform v1.16.1 (linux_amd64). Låst i `.terraform.lock.hcl`:
`hetznercloud/hcloud 1.66.0`, `aminueza/minio 3.12.0`, `jackivanov/x25519 1.0.8`,
`clementblaise/age 0.1.1`, `hashicorp/tls 4.4.0`, `hashicorp/local 2.9.0`.
`hcloud` og `minio` er eksakt pinnet i `providers.tf`; `x25519` og `age` bruker `~>`;
`tls` og `local` er upinnet i upstream og ble løst til nyeste.

Modulen `sops` ble hentet fra `file:///opt/infra-tools/shared-modules/sops`, altså den
read-only lokale mounten. `terraform validate`: Success. `terraform fmt -check`: ren.

### Planresultat: 12 to add, 0 to change, 0 to destroy

| Ressurs | Konkret virkning |
| --- | --- |
| `tls_private_key.generated` | RSA 4096 admin-nøkkelpar, genereres lokalt |
| `hcloud_ssh_key.generated` | Registrerer public key i Hetzner Cloud som `fau-admin` |
| `local_file.private_key_file` | `../.config/ssh_key/id_ed25519`, 0600 (RSA, tross filnavnet) |
| `local_file.public_key_file` | `../.config/ssh_key/id_ed25519.pub`, 0644 |
| `minio_s3_bucket.k3s_backup_bucket` | Privat bøtte `fau-k3s-backup` i hel1, `force_destroy = true` |
| `minio_s3_bucket.db_backup_bucket` | Privat bøtte `fau-db-backup` i hel1, `force_destroy = true` |
| `x25519_private_key.vpn_router` | WireGuard-nøkkelpar, lokalt |
| `x25519_private_key.devops` | WireGuard-nøkkelpar, lokalt |
| `x25519_private_key.remote_router` | WireGuard-nøkkelpar, lokalt |
| `module.sops.age_secret_key.sops` | age-nøkkelpar til SOPS, lokalt |
| `module.sops.local_file.age_sops_private_key` | `../.sops.yaml`, 0600 — kun age-**public**-nøkkelen |
| `local_file.persistent_output` | `../.config/persistent_outputs.json`, 0600 |

Ingen VM, dedikert server, load balancer, floating IP, nettverk eller OS opprettes.
De eneste eksterne ressursene er én SSH-nøkkelregistrering (gratis) og to S3-bøtter.
Tomme bøtter koster i praksis ingenting; kostnad følger lagret volum og trafikk senere.

`force_destroy = true` på begge bøttene er upstream-adferd og er beholdt: en framtidig
`terraform destroy` ville da slette backupinnhold uten å stoppe. `prevent_destroy` er
utkommentert hos upstream. Vurder å slå det på når bøttene faktisk inneholder backup.

Planen er lagret som `stage0.tfplan` i modus 0600. Den inneholder variabelverdiene, altså
credentials, og skal ikke kopieres ut av runtime-volumet eller vises i chat/Favro.

### Credentials er verifisert utenom planen

Steg 0 har ingen data sources, så en vellykket plan beviser ikke at nøklene virker.
Kontrollert separat, read-only, uten å skrive ut verdier:

- Hetzner Cloud API `GET /v1/ssh_keys` → HTTP 200. Prosjektet har ingen SSH-nøkler fra før,
  så `fau-admin` kolliderer ikke.
- Hetzner object storage `GET /` (ListBuckets, SigV4) mot `hel1.your-objectstorage.com`
  → HTTP 200, ingen bøtter fra før. Verken `fau-k3s-backup` eller `fau-db-backup` kolliderer,
  og S3-nøklene er gyldige for hel1.

Begge målene er altså tomme: dette er en førstegangsoppretting, ikke en gjenbruk av
eksisterende state. Ingen tidligere state ble funnet eller overskrevet.

### Gjenstår

Apply er ikke kjørt og er ikke autorisert av dette dokumentet. `ALLOW_TERRAFORM_MODIFY`
står på `Ask`. Etter en eventuell apply kopieres kun `.sops.yaml` tilbake til `/workspace`.

## Apply utført — 8. september 2026

Erik autoriserte apply i chat. Kjørt som `terraform apply stage0.tfplan` fra
`/infra-runtime/infrastructure/0-persistent` med `umask 077`.

**Resultat: 12 added, 0 changed, 0 destroyed.**

### Opprettede eksterne ressurser

| Ressurs | Identitet |
| --- | --- |
| Hetzner Cloud SSH-nøkkel | id `118512502`, navn `fau-admin`, MD5 `6d:ac:3e:7b:4e:dc:52:0d:5b:33:28:58:6a:0a:95:e8` |
| S3-bøtte (hel1) | `fau-k3s-backup`, privat |
| S3-bøtte (hel1) | `fau-db-backup`, privat |

Verifisert etterpå mot API-ene, read-only: `GET /v1/ssh_keys` viser `fau-admin` med
fingerprint som stemmer med `ssh-keygen -E md5` på den lokale public key-filen, og
ListBuckets mot hel1 viser begge bøttene. `terraform plan` kjørt på nytt etterpå:
«No changes. Your infrastructure matches the configuration.» (exitcode 0).

### Genererte lokale filer

Alle i `/infra-runtime/infrastructure`, som er 0700 `claude:claude`:

- `.config/ssh_key/id_ed25519` — RSA 4096 privatnøkkel, 0600
- `.config/ssh_key/id_ed25519.pub` — public key, 0600
- `.config/persistent_outputs.json` — 0600, handoff til steg 1/2
- `.sops.yaml` — 0600
- `0-persistent/terraform.tfstate` — 0600

Merk: `id_ed25519.pub` ble 0600, ikke 0644 som planen anga. `local_file` skriver med
`os.WriteFile`, så prosessens `umask 077` maskerer bort group/other. Dette er strengere enn
konfigurasjonen ber om og gir ingen drift ved ny plan. Filnavnet er fortsatt misvisende:
nøkkelen er RSA, ikke Ed25519.

`.sops.yaml` er kopiert til `/workspace/.sops.yaml` (0644). Den inneholder kun
age-public-nøkkelen `age135aql83qcmr2n4haspdmwwq6vlw35rm083jgr3scwdk7hnt5xuvq2y0ytl` og er
kontrollert for at den ikke inneholder hemmeligheter før kopiering. Dette er den eneste
genererte filen som forlater runtime-volumet. age-**privat**nøkkelen ligger i
`persistent_outputs.json` og i state, begge i runtime-volumet.

### Konsekvenser å være obs på

- `terraform.tfstate` inneholder nå alle privatnøkler i klartekst. Den ligger kun i
  `infra-runtime`-volumet. `docker compose down -v` ville slette den. Ikke bruk `-v`.
- Ingen backup av state er satt opp. Det er nå den mest verdifulle filen i oppsettet.
- `force_destroy = true` står fortsatt på begge bøttene.
- `stage0.tfplan` er brukt opp og kan slettes; den inneholder credentials.
- Steg 0 opprettet ingen VM, dedikert server, LB eller OS. Neste steg krever at
  privatnett/VPN er avklart fra agentmiljøet, jf. S12 i pre-0-guiden.

## Bøttebeskyttelse slått på — 8. september 2026

Erik valgte «better safe than sorry». `storage.tf` i FAU sin root har nå `prevent_destroy = true`
og `force_destroy = false` på begge bøttene. Upstream har motsatt oppsett: `force_destroy = true`
med lifecycle-blokken utkommentert.

`prevent_destroy` får Terraform til å avvise enhver plan som ville slette **eller erstatte**
bøtta, inkludert `terraform destroy` på hele steget. `force_destroy = false` er andre skanse:
Terraform nekter å slette en bøtte som fortsatt har objekter, dersom `prevent_destroy` en gang
fjernes.

Applied: 0 added, 2 changed, 0 destroyed (`force_destroy true -> false` in-place på begge).
Etterpå gir `terraform plan` «No changes». Kontrollert at vernet virker: `terraform plan -destroy`
feiler nå med «Instance cannot be destroyed» for begge bøttene. Det var en plan, ingenting ble slettet.

Skal en bøtte pensjoneres bevisst: fjern lifecycle-blokken og sett `force_destroy = true` i samme
endring, kjør plan og les den før apply.

`stage0.tfplan` beholdes som revisjonsspor etter Eriks valg. Den ligger 0600 i runtime-volumet og
inneholder variabelverdiene, altså credentials. Den skal ikke kopieres ut eller vises. Merk at den
nå er utdatert mot faktisk state — den beskriver førstegangsopprettingen, ikke dagens konfigurasjon.

## Undersøkt: remote state i Hetzner object storage — 8. september 2026

Testet fra agentmiljøet med signerte SigV4-kall mot `hel1.your-objectstorage.com` for å avgjøre
om Terraform sin S3-backend er et reelt alternativ til lokal state. Backend er Ceph RGW
(RequestId-suffiks `hel1-prod1-ceph4`).

### Betingede skrivinger virker — `use_lockfile` er brukbart

Terraform 1.10+ låser state med S3-native betingede skrivinger (`If-None-Match: *`) i stedet for
DynamoDB. Testet mot den etablerte bøtta `fau-k3s-backup` med et midlertidig objekt som ble
slettet etterpå:

- første `PUT` med `If-None-Match: *` → 200
- andre `PUT` med `If-None-Match: *` → **412 Precondition Failed**

412 er det svaret låsemekanismen krever. `use_lockfile = true` vil altså fungere; ingen
DynamoDB-erstatning er nødvendig.

### Versjonering støttes

`PUT ?versioning` med `Status: Enabled` → 200, og `GET ?versioning` svarer
`<Status>Enabled</Status><MfaDelete>Disabled</MfaDelete>`. API-ene for `lifecycle`, `encryption`
og `object-lock` finnes også (de svarer «not configured», ikke «not implemented»).

Ikke bekreftet: at flere versjoner faktisk beholdes ved gjentatte skrivinger. Hvert forsøk ble
avbrutt av propageringsproblemet under, ikke av versjonering som sådan. Må verifiseres på den
faktiske state-bøtta etter at den er opprettet og har satt seg.

### Viktig funn: nye bøtter propagerer tregt og ujevnt

En nyopprettet bøtte er ikke umiddelbart brukbar. `PUT /bucket` svarer 200, men påfølgende
forespørsler veksler mellom 200 og **404 NoSuchBucket** i minuttene etterpå — målt seks
overskrivinger på rad som ga 404, 200, 404, 200, 404, 404. Første vellykkede skriving kom
etter 9–15 sekunder, og ustabiliteten fortsatte etter det.

Dette ser ut som at bøttemetadata ikke er propagert til alle gateway-noder bak lastbalansereren.
Etablerte bøtter (`fau-k3s-backup`) oppfører seg helt stabilt.

Konsekvens: opprett state-bøtta som et eget, tidlig steg, vent til den svarer stabilt, og
verifiser før `terraform init -migrate-state`. Ikke opprett bøtta og migrer state i samme
operasjon. Det samme gjelder om senere Terraform-kode oppretter en bøtte og skriver til den
umiddelbart.

### Ryddet opp

Testbøtta `fau-tmp-versioncheck-8sep` og alle testobjekter er slettet. Kontrollert over åtte
pollinger at kun `fau-db-backup` og `fau-k3s-backup` finnes, og at direkte oppslag på testbøtta
gir 404. Ingen Terraform-state ble berørt; testbøtta ble aldri lagt inn i state.

### Gjenstår å avgjøre

Om remote state gjelder bare steg 0 eller alle fire stegene. Infra-tools bruker lokal state
overalt, og steg 1–3 leser `persistent_outputs.json`, ikke remote state, så steg 0 kan flyttes
alene. State ligger uansett i klartekst i bøtta; tilgangen styres av de samme S3-nøklene som
allerede ligger i `.credentials.tfvars`.

## Remote state migrated and destruction-proofed — 9 September 2026

Two things are recorded here that the previous sections did not: the migration to remote state
actually happened on 8 September, and the protections around it were verified on 9 September.

### Remote state is live (8 September 2026, 12:56 UTC)

Stage-0 state no longer lives in the `infra-runtime` volume. `backend.tf` in FAU's own root
configures Terraform's S3 backend against the `fau-tfstate` bucket, key
`0-persistent/terraform.tfstate`, region `hel1`, endpoint `https://hel1.your-objectstorage.com`,
with `use_path_style = true` and `use_lockfile = true`. Credentials are not in the file: they are
passed with `terraform init -backend-config=/infra-runtime/infrastructure/.backend.hcl`, mode
0600, which never leaves the runtime volume.

The bucket is deliberately **not** managed by Terraform — a bucket cannot hold the state that
manages it — so it must never be added to any root configuration. It was created out of band.
The local `terraform.tfstate` is now 0 bytes and the pre-migration copy is kept as
`terraform.tfstate.pre-remote-8sep2026` in the runtime volume. `terraform state list` reads all
twelve stage-0 resources through the backend.

Consequence worth noting: because state holds the admin SSH key, the three WireGuard keys and the
age key in clear text, those secrets are now in Hetzner object storage as well as the volume. The
S3 keys that gate them are the same ones already in `.credentials.tfvars`.

### Versioning provably retains versions

`GET ?versioning` reports `Enabled`. Retention itself was verified on 9 September against a
scratch key `_versioncheck/probe`, never the state key: two overwrites left three versions
coexisting, a plain `DELETE` produced a delete marker with `GET` then returning 404 `NoSuchKey`,
and `GET ?versionId=` returned the prior content byte for byte. All three versions and the delete
marker were then purged and the bucket verified back to exactly one object, the state, with its
ETag, size and LastModified unchanged.

This closes the "ikke bekreftet" item from the investigation section above.

**A read timeout is not a failed write.** Two PUTs of 8 and 20 bytes exceeded a 30-second read
timeout and both landed server-side regardless. Do not infer from a timeout that a state write
failed, and do not retry blindly: with `use_lockfile` the retry presents as a lock conflict
rather than as the completed write it actually is.

### The state object carries a null version

The single version of `0-persistent/terraform.tfstate` has `VersionId: null`, because it was
written before versioning took effect. Overwrite behaviour for a null version could not be
tested on a fresh key, since versioning is now enabled and every new key receives a real id.

Rather than rely on the assumption, a server-side copy was taken to
`_snapshots/2026-09-08T125636Z-terraform.tfstate`. It has a real version id
(`ay9MpumXMxI-DZg8MWHSM16RremuAsn`) and the same ETag `8bb410f52d4850ba7751922b582b8310` and
size 35674 as the source, so the 8 September state exists as an independently versioned object.

### Bucket policy blocks permanent destruction

A bucket policy is now set on `fau-tfstate`, id `fau-tfstate-no-accidental-destruction`. It is an
explicit `Deny` for `Principal: "*"` on:

- `s3:DeleteObjectVersion` on `arn:aws:s3:::fau-tfstate/*` — the only operation that destroys
  data permanently
- `s3:DeleteBucket` and `s3:PutBucketVersioning` on the bucket itself

The deployed policy is kept verbatim in the repository at
`infrastructure/fau-tfstate-bucket-policy.json`, verified identical to what the bucket returns.
It is documentation and a reproduction source, not Terraform input — the sync allow-list does not
touch it, because the bucket is not Terraform-managed.

`s3:DeleteObject` stays allowed on purpose: with versioning on it only creates a recoverable
delete marker, and Terraform's lock release needs it. `s3:PutBucketPolicy` is deliberately **not**
denied, so the guard remains removable. That is the intent — removing it takes a deliberate policy
change, not an accident.

Verified against our own owner credential, which is the case that matters, since Hetzner issues no
second credential with narrower rights:

- versionId delete on a non-existent key → `403 AccessDenied`, so the deny is evaluated before the
  existence check and no object had to be risked to prove it
- `PUT ?versioning` with the identical value `Enabled` → `403 AccessDenied`
- `terraform plan` still works: lock acquired, twelve resources refreshed, "No changes", lock
  released

`DELETE` on the bucket returned `409 BucketNotEmpty`, not 403 — and from a different request-id
format than the Ceph responses, so that path appears to be answered ahead of the policy engine.
The `s3:DeleteBucket` deny is therefore **unverified**. Bucket deletion is still blocked, but
transitively: emptying the bucket requires `DeleteObjectVersion`, which is denied.

### Accepted cost: lock versions accumulate

Each Terraform run leaves a 229-byte `terraform.tfstate.tflock` version plus a delete marker, and
those can no longer be purged, because purging them is exactly the denied operation. They are
invisible to a normal listing — the live listing shows only the state and the snapshot — and the
volume is kilobytes per thousand runs, so this is accepted. Clearing them would mean lifting the
policy, purging, and re-applying it. A lifecycle rule might do it unattended, but whether the
lifecycle processor is itself subject to the policy deny is untested.

### Decided: no copy outside Hetzner

State is protected against accident, not against loss of the Hetzner account or project: one
provider, one project, one credential pair, and no copy outside Hetzner. Erik decided on
9 September that this is sufficient — the keys in state can be regenerated from the Hetzner
console, and losing the account would take the infrastructure with it. Proton Pass is the
intended future home for a manual key export.

The decision holds while nothing consumes the generated keys. Registering the admin public key in
Hetzner Robot is **not** a trigger: Robot is reachable through its web interface, its API
credentials and the rescue system, any of which can install a replacement key, so the admin SSH
key stays regenerable. The one trigger is GitOps secrets encrypted to the SOPS age key, because a
regenerated age key cannot decrypt what the old one encrypted. Storing the keys in Proton Pass
before that point removes the trigger. See docs/planning-decisions.md, "No off-Hetzner state copy
required".
