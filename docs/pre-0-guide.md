# Pre-0: eget FAU-prosjekt med agent-box — #3438

Revidert 7. september 2026. Erstatter utkastet med FAU inne i infra-tools-checkouten. Kryss av stegene på Favro-kortet etter faktisk utførelse.

## Valgt forslag og ansvar

Bruk et eget FAU-prosjekt med agent-box som agent-tjeneste og senere FAU-app og PostgreSQL som egne tjenester i samme Compose-stack. Agenten kan bygge, teste og lese logger gjennom Docker-socketen og nå appen via tjenestenavn. Appcontaineren trenger ikke Terraform eller leverandørnøkler.

Infra-tools sine delte moduler beholdes uendret. FAU trenger egen Terraform-rootkonfigurasjon for navn, lokasjon, modulreferanse, nodetopologi og lagring. Path/Git-valget er Terraform sitt infra_tools_path/infra_tools_ref, ikke en funksjon i demo-app. Demo-app-containeren kjører Django, mens agent-box er utviklings-/administrasjonsmiljøet.

Pakken har agent-service, instruksjoner og prosjektkontekst. Ingen appimplementasjon, Terraform-apply, credentials eller state følger med. Agenten lager FAU-konfigurasjonen etter oppstart.

## S1 — Opprett egen mappe på Docker-verten

Kontroller `docker version` og `docker compose version` på maskinen som kjører Docker. Lag en egen FAU-mappe, eksempel `fau-platform`. Last ned vedlagte `fau-pre-0-handoff.zip` og pakk den ut der, ikke inne i infra-tools. Behold eksisterende filer hvis prosjektmappen allerede finnes.

## S2 — Finn infra-tools-checkouten

Bruk eksisterende checkout eller hent det private repoet på verten med din GitHub-tilgang. Inspisert commit er `6c301e15e3ee83a9797971046ffe1bc9b8593ca3`, version-4. Kontroller faktisk versjon og bevar lokale endringer. Ingen egen agent-box-checkout er nødvendig når ferdig image brukes.

## S3 — Fyll ut Compose-miljøet

Kopier `.env.example` til `.env` i FAU-mappen. Sett FAU_HOST_PATH til absolutt FAU-sti og INFRA_TOOLS_HOST_PATH til infra-tools-sti, slik Docker-daemonen på verten ser dem. Sett eget TTYD_PASSWORD. Bruk Docker Desktop/WSL-kompatible stier der relevant. Ingen Hetzner-/Favro-/OpenAI-nøkler skal i denne filen.

Pakken bruker localhost-portene 8112/8113 og Compose-navnet fau-mvp. Det unngår kollisjon med infra-tools sine 8102/8103. Docker-socket gir omfattende kontroll over verten; behold terminalene lokalt eller bak sikker tunnel. Ikke del en full `docker compose config`-utskrift, som inkluderer terminalpassordet.

Eksplisitte hoststier gjør at senere Compose-kjøring inne i agenten treffer samme bind-mount på verten. Nye app-tjenester bruker for eksempel `${FAU_HOST_PATH}/backend` som mount-kilde, ikke containerens `/workspace/backend`. Agenten kan bruke /workspace som byggekontekst; Docker-klienten sender byggfiler til daemonen. Bruk named volumes til DB-data.

## S4 — Start agent-box og logg inn i Claude

Fra FAU-mappen på verten: `docker compose up -d agent`, deretter `docker compose ps`. Dette starter kun administrasjonscontaineren, ikke Hetzner-servere. Åpne http://localhost:8112, velg New session og fullfør Claude-innloggingen. For fjernvert må begge porter 8112/8113 tunneleres. Pakken bruker agent-box:v1.4.2, som matcher inspisert tag; faktisk imageinnhold kontrolleres i S6.

Claude er nok for å komme videre. Codex er valgfritt. Behold sesjonsfanen åpen mens agenten arbeider; agent-box avslutter Claude-prosessen når fanen lukkes og beholder historikken i volumet.

## S5 — Gi agenten privat runtime-lagring

Kjør én gang fra verten:

`docker compose exec --user root agent install -d -m 0700 -o claude -g claude /infra-runtime/infrastructure`

FAU-filer er montert på /workspace, infra-tools read-only på /opt/infra-tools. /infra-runtime er et eget vedvarende volum for Terraform-kjøreroter, state, credentials og genererte nøkler. Dette følger agent-box-manualens regel om å holde slike hemmeligheter utenfor workspace. Behold også egne Claude-/Codex-volumer. Ikke bruk `down -v`. State og nøkler må bevares selv om omfattende SSD-backup av testserveren er utsatt.

## S6 — La agenten kontrollere verktøy og tilgang

Gi agenten overleveringsteksten nederst. Den skal kontrollere bruker, Terraform-versjon, Docker/Compose, Node/npm, Python, kubectl og tilgjengeligheten av Rust/cargo, Codex, Favro, SOPS og WireGuard.

Inspisert agent-box-Dockerfile installerer Claude, Node 22, Terraform, kubectl, Docker CLI, SSH, Python og Playwright. Rust, Codex, SOPS og WireGuard er ikke eksplisitt installert. Terraform må være >=1.15.0; Docker Compose-plugin må faktisk virke. README sier Compose støttes, men Dockerfile installerer ikke plugin eksplisitt. Ikke anta at verktøy mangler bare fordi de manglet i den gamle containeren.

## S7 — Bygg nødvendige tillegg ved behov

La agenten skrive en FAU-eid Dockerfile som utvider agent-box:v1.4.2. Behold entrypoint og Claude-funksjoner. Legg til Rust/cargo med C-kompilator/linker og nødvendige biblioteker for FAU/Favro. Node finnes allerede. Oppgrader Terraform/installer Compose-plugin bare ved påvist behov. Pin og dokumenter valgte verktøyversjoner. Rust er ikke nødvendig for Terraform selv, men Favro CLI-kilden krever Rust >=1.82.

Agenten leverer Dockerfile og Compose-endring. Du bygger/reoppretter agent-tjenesten fra verten når de er klare. Agent-box-manualen forbyr agenten å restarte/rebygge sin egen tjeneste. Dette håndgrepet gjelder imageendringer; agenten kan senere bygge og styre app/db selv. Ingen nye volumer skal slette eksisterende innlogging/state.

## S8 — Installer/logg inn i Codex hvis ønsket

Codex kan legges til i det utvidede imaget med offisiell Linux-installasjon. Det er ikke integrert i agent-box sin Claude-sesjonsside. Fra verten åpner du `docker compose exec --user claude -w /workspace agent bash`, deretter `codex login --device-auth` og `codex login status`. Fullfør nettleserflyten hvis kontoen tillater device code; start `codex` i /workspace. Eget .codex-volum bevarer innloggingen. Brukes Claude alene, kryss av som «ikke nødvendig nå».

Kilder: https://learn.chatgpt.com/docs/codex/cli og https://learn.chatgpt.com/docs/auth. ALLOW_TERRAFORM_MODIFY er en Claude-hook, ikke en sikkerhetskontroll for Codex. Begge følger FAU-instruksene om konkret plan før apply. Compose beholder agent-box-standard Ask.

## S9 — Koble Favro til samme FAU-prosjekt

Agenten leser AGENTS.md, CLAUDE.md, docs/planning-decisions.md og .agents/skills/favro/SKILL.md. Hvis CLI mangler, installer etter Rust-oppsett med `cargo install --path /workspace/.agents/skills/favro/favro-cli --locked` og kontroller favro >=0.2.0.

Legg Favro-credentials lokalt i `/infra-runtime/infrastructure/favro.env`, modus 0600. Pakkens .favro/project.toml peker dit og til eksisterende FAU-plattform. Kjør `favro check`. Ikke opprett ny collection. Outbox kan brukes dersom Favro-tilgang må ordnes mens lokal klargjøring fortsetter.

## S10 — Klargjør FAU sin Terraform-root

Agenten kopierer bare steg-0-kildefiler fra /opt/infra-tools/infrastructure/0-persistent til /workspace/infrastructure/0-persistent og tilpasser FAU-navn/lokasjon. Sett infra_tools_path til /opt/infra-tools og infra_tools_ref til tom streng. Delte moduler forblir uendret. Alternativt brukes Git-kilde med commit-festet ref; det krever GitHub-lesetilgang ved init. Lokal mount er enklest i første oppsett.

Agenten synkroniserer kun navngitte kildefiler til /infra-runtime/infrastructure/0-persistent og kjører Terraform der. State, credentials og genererte filer skal aldri overskrives av synkroniseringen. Relative outputs havner da i /infra-runtime/infrastructure/.config. Opprett .config og .config/ssh_key med modus 0700 på forhånd; `local_file` lager ellers mapper med 0777. Rettelse 8. september 2026: upstream kaller sops-modulen med `sops_filename = "../../.sops.yaml"`, som fra stegmappen peker på /infra-runtime/.sops.yaml. Den mappen er root:root 0755, og agenten kjører som claude, så apply ville feilet. FAU sitt rotkall bruker derfor `"../.sops.yaml"`, altså /infra-runtime/infrastructure/.sops.yaml. Den delte modulen er uendret. Bare offentlig SOPS-konfigurasjon kan kopieres tilbake til FAU-prosjektet. Kontroller modulstier og filrettigheter. Eksisterende state fra tidligere kjøring må bevares. hel1 er valgt lokasjon (Erik, 8. september 2026) og må passe faktisk S3-lokasjon.

## S11 — Legg inn Hetzner-credentials

Opprett/velg Cloud-prosjekt, read/write API-token og S3 access/secret key. Lag `/infra-runtime/infrastructure/.credentials.tfvars` med 0600. Fyll hcloud_token, s3_access_key og s3_secret_key. Agenten kan generere grafana_admin_password direkte i filen uten å vise det. Robot API-bruker/passord kan legges inn nå eller før dedicated-steget. Nøkkelverdier skal aldri i Favro/chat.

S3 beholdes. Det er omfattende SSD-backup som er nedprioritert. Steg 0 genererer SSH-nøkkelparet og registrerer public key i Cloud; samme public key registreres senere i Robot. Ingen separat nøkkel eller serverreset er nødvendig nå.

## S12 — La agenten overta init, validate og plan

Agenten kjører fra /infra-runtime/infrastructure/0-persistent med `umask 077`: init, validate og `terraform plan -input=false -var-file=../.credentials.tfvars -out=stage0.tfplan`. Den registrerer faktiske verktøyversjoner og planresultat uten hemmeligheter på #3438/#3424. Ressursendringene legges fram før apply. Steg 0 oppretter ingen OS eller VM.

Før senere steg 1/2 må privatnett/VPN verifiseres fra agentmiljøet. Compose-oppsettet gir ikke automatisk TUN/NET_ADMIN eller VPN-ruting. Velg dette konkret når nettverket foreligger; Docker-sockettilgang alene beviser ikke privatnett-tilgang. Det hindrer ikke steg 0.

## Overleveringstekst til agenten

«Les AGENTS.md, CLAUDE.md, docs/pre-0-guide.md og siste planning-decisions.md. Fortsett Favro #3438 i dette FAU-prosjektet. Kontroller verktøy og lag nødvendige tillegg i en FAU-eid Dockerfile. Jeg gjør nødvendig rebuild av agenten fra verten. Gjenbruk /opt/infra-tools sine moduler uendret og lag FAU-eide Terraform-roter. Hold credentials/state/genererte nøkler i /infra-runtime/infrastructure. Installer og bruk Favro-skillen mot eksisterende prosjekt. Kjør init/validate/plan når credentials er på plass, og vis konkrete endringer før apply. App og DB skal være tjenester i denne Compose-stacken. S3 beholdes, omfattende SSD-backup utsettes, fysisk vert som felles feilpunkt er akseptert og enkel senere flytting er prioritert. Steg 0 lager SSH-nøklene. Ikke kjør demo-apply eller reinstallasjon av serveren.»

## Evidens og status

Agent-box commit `1816d1e6b7ac1c682c294ac5ee3558107c3d15f4`, tag v1.4.2, ble hentet og lest: https://github.com/akantodevs/agent-box/blob/1816d1e6b7ac1c682c294ac5ee3558107c3d15f4/README.md samt agent-box/Dockerfile og agent-box/CLAUDE.md i samme commit. Infra-tools Compose og demo_app Compose/Dockerfile er kontrollert lokalt. Image-build, containerstart, VPN og Terraform-plan er ikke kjørt her. Kryss av etter faktisk utførelse.
