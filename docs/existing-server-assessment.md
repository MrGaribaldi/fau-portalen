# Eksisterende Hetzner-server som MVP-vert

7. september 2026. Foreløpig anbefaling, eid av Infrastruktur, drift og lagringsøkonomi, #3408; relevant for #3411/#3424/#3425.

## Konklusjon

Det er teknisk plausibelt å kjøre master og worker som to VM-er på Eriks eksisterende dedikerte maskin. 64 GB RAM gir et godt utgangspunkt for et lite MVP-oppsett, men CPU, diskhelse, I/O, nettverk og hele telemetry-stakken er ikke målt. To VM-er er to logiske Kubernetes-noder på ett fysisk feilpunkt. Dette er en kostnadsmodell, ikke uavhengig redundans.

Maskinvare er oppgitt av Erik: i7-6700, 64 GB RAM, 2 × 512 GB NVMe SSD. Ingen SSH, reinstallasjon, Terraform apply, VM-opprettelse eller bestilling er utført. Dagens OS, virtualiseringsstøtte i oppsettet, diskmodell/-helse, lokasjon, pris og brukbar ledig kapasitet er ukjent.

Senere avklaring: dagens OS er openSUSE Leap. Erik er villig til å reinstallere til passende OS. Backup har lavere prioritet for denne MVP-testen; tidligere formuleringer om obligatorisk ekstern restore før oppstart gjelder derfor ikke som inngangskrav til testen. #3425 beholdes som senere arbeid. Enkel flytting videre er prioritert: samme appimage, deklarativ konfigurasjon, PostgreSQL med versjonerte migreringer, parameterisert storageklasse og en dokumentert eksport/import-vei. En flytteprøve med testdata skal kontrollere dokumenter, tenanttilgang og historikk uten å kreve en komplett produksjonsberedskap nå.

OS-forslag: Ubuntu 24.04, fordi `shared-modules/cloud-server/main.tf:9` bruker dette og dedicated-modulen forventer Netplan. Kontroller tilgjengelig installimage-image og faktiske disker/nettverksinterface før en konkret installasjonskonfigurasjon lages. Hetzner Rescue → installimage → omstart til installert OS → infra-tools worker-bootstrap er den verifiserte oppdelingen. En vanlig reset alene installerer ikke OS. Installimage støtter `/autosetup`, så OS-steget kan automatiseres med Hetzners eksisterende verktøy; denne automasjonen er ikke funnet i infra-tools-checkouten. Ikke kjør dedicated-modulens Netplan-konfigurasjon direkte i Rescue og forvent et varig installert operativsystem.

## Hva koden faktisk støtter

Kilde: lokal infra-tools, commit `6c301e15e3ee83a9797971046ffe1bc9b8593ca3`. Inspeksjon, ikke kjøretest.

Oppfølging: Erik har uttrykkelig akseptert felles fysisk feilpunkt i denne fasen uten betalende kunder. Dette er ikke lenger en åpen godkjenningssak. Prioriter gjenbruk. `readme.md:611–620` beskriver nettopp dedicated CPU/GPU-workers med ekstern SSH-adresse, vSwitch og samsvarende Robot-/nodenavn. Forfatterens beskrivelse stemmer med worker-bootstrap; samme mekanisme er relevant for en vanlig CPU-server. De lokalt tilgjengelige origin/version-2 og origin/version-3 beskriver også CPU/GPU-bruken. Ingen ny fetch er utført.

Den generiske `shared-modules/k3s-agent` støtter server-IP, SSH-nøkkel/-port, Robot-ID og overstyring av nettverksinterface. Den er en konkret gjenbruksvei etter OS-/nettverksoppsett. `remote-server` i denne checkouten oppretter derimot en ny hcloud-server før WireGuard-oppsett; det er ikke en ferdig importer for eksisterende maskiner. VM-er er et mulig alternativ, ikke et krav for å bruke dedicated-serveren som worker. Å erstatte cloud-worker direkte gir mest umiddelbart gjenbruk; å erstatte master også krever egen tilpasning. Avklar dette skillet før en hypervisor innføres.

- `shared-modules/dedicated-server/main.tf`: forutsetter SSH-nøkkel registrert i Robot og installert server. Skriver Netplan-konfigurasjon og installerer K3s **agent**. Ingen Rescue-aktivering, OS-installering eller hypervisor-/VM-opprettelse funnet i denne modulen eller søket i checkouten.
- `shared-modules/dedicated-server/variables.tf`: krever mer enn IP/nøkkel: Robot-ID, node-IP, subnett, VPN-router og K3s-server/token.
- `shared-modules/dedicated-server/netconfig.tpl.yaml`: hardkoder `enp6s0`, VLAN 4000, gateway `10.0.2.1` og MTU 1400. Dette må tilpasses faktiske grensesnitt/nettverk før bruk. SSH-/bastionkoblinger er heller ikke konsistente mellom alle provisioners og må gjennomgås.
- `infrastructure/1-bootstrap/configuration.tf` og `controlplane.tf`: standarden oppretter VPN-router, cloud-master og cloud-worker, pluss separat LB11. vSwitch-ID er 0, altså ikke konfigurert. Master er fortsatt eksplisitt en cloud-server. Dedicated-modulen er ikke koblet inn i denne standardens worker-opprettelse.
- `shared-modules/dedicated-server/k3s-agent-config.tpl.yaml`: bruker fysisk Robot-ID som provider-ID. To egne VM-er skal ikke ukritisk gjenbruke samme Robot-ID som om de var to fysiske servere. Identitet, CCM-håndtering og LB-targets krever et eget VM-oppsett.
- `shared-modules/bootstrap/cluster/csi.tf`: Hetzner CSI er begrenset til noder med `cloud=true`. PostgreSQL, RabbitMQ, Grafana og deler av telemetry bruker `hcloud-volumes` som standard; noen navn er hardkodet, andre parametriserte.

Hetzner støtter SSH til Rescue og OS-installasjon med installimage. Rescue må aktiveres og serveren rebootes; dette er ikke det samme som at infra-tools automatiserer hele flyten. [Rescue](https://docs.hetzner.com/robot/dedicated-server/troubleshooting/hetzner-rescue-system/), [installimage](https://docs.hetzner.com/robot/dedicated-server/operating-systems/installimage/).

## To aktuelle modeller

| Modell | Gevinst | Tilpasning og konsekvens |
| --- | --- | --- |
| To VM-er på eksisterende vert: master/worker | Kan erstatte begge betalte cloud-arbeidsnodene hvis verten uansett beholdes | Krever hypervisor, VM-provisionering, nettverk/CCM og lokal storageklasse. Begge noder og begge eventuelle PG-kopier deler maskinfeil. VPN-router/LB må fortsatt regnes inn. |
| Cloud-master + eksisterende fysisk server som worker | Erstatter cloud-worker med betydelig tilgjengelig RAM og følger dedicated-modulens hensikt tettere | Beholder cloud-masterkostnad; vSwitch, dedicated-bootstrap og lagringsplassering må tilpasses. Gir to fysiske verter, men ikke automatisk HA for alle tjenester. |

Foreslått retning ved maksimal reduksjon i nye månedlige kostnader er VM-modellen, dersom Erik aksepterer planlagt vedlikeholdsnedetid og risikoen ved én vert. Hybridmodellen er et alternativ med mindre endring i control-plane-provisioneringen. Velg hypervisor først etter avklaring av nåværende OS; det finnes ingen ferdig KVM/Proxmox-provisionering i undersøkt kode.

Foreløpig VM-budsjett for å teste, ikke kapasitetsgaranti: 16 GB til master med workloads, 32 GB til worker, 16 GB beholdt for vert og margin. CPU deles på den ene prosessoren og må lasttestes; antall tildelte vCPU-er skaper ikke flere fysiske kjerner. RAID1/speiling av to 512 GB-disker gir omtrent én disks kapasitet før OS, filsystem og reserve, ikke 1 TB brukbar kapasitet. To VM-disker og to DB-kopier bruker det samme underliggende diskparet. Speiling erstatter ikke ekstern backup.

## Lagring og nettverk er de viktigste endringene

Hetzner Cloud Volumes kan ikke kobles til dedikerte servere. Derfor er det utilstrekkelig bare å endre nodenes IP-adresser: en eksplisitt lokal storageklasse/PV-strategi og plassering av alle stateful workloads må erstatte cloud-volume-avhengighetene på disse nodene. [Hetzner Volumes FAQ](https://docs.hetzner.com/cloud/volumes/faq/).

Lokal lagring skal ha dokumentert node-affinity, gjenoppretting og kapasitetsovervåkning. Databasekopier må ha separate kataloger/VM-disker. Dagens krav om ulike Kubernetes-hostnames kan tilfredsstilles av to VM-er, men gir ingen beskyttelse mot tap av den fysiske verten. Ikke markedsfør dette som HA. Ekstern PostgreSQL/WAL-/etcd-backup og isolert restore må bevises før kundedata legges inn.

Load Balancer kan bruke dedikerte serveres offentlige/private IPv4. Private targets krever felles Cloud Network/vSwitch, og dedikert server må tilhøre samme eier/Robot-konto; IP-targets støttes i eu-central. VM-adresser og automatisk CCM-registrering er ikke dermed verifisert. Planlegg og test LB → ingress → service med korrekt ruting/MTU/brannmur. [Hetzner Load Balancer FAQ](https://docs.hetzner.com/networking/load-balancers/faq/), [vSwitch-kobling](https://docs.hetzner.com/networking/networks/connect-dedi-vswitch/).

## Kostnad og senere flytting

Hvis den dedikerte serveren betales og beholdes uansett, kan VM-modellen unngå to nye cloud-nodeutgifter og deler av volumeutgiftene. Hvis alternativet er å si opp serveren, skal hele serverleien regnes med. Ingen konkret besparelse fastsettes før månedlig leie, behold/si-opp-alternativ og nødvendige tillegg er kjent. VPN-router, LB, ekstern backup/S3, e-post og eventuelle IP-/trafikkostnader inngår fortsatt.

Appen bruker samme image og deploykontrakt fra #3411. Senere flytting krever likevel dataarbeid: opprett målcluster eller nye noder, provisioner riktige PVC-er, flytt PostgreSQL gjennom testet replikering eller backup/restore, avklar øvrige stateful tjenester, verifiser tenantdata/historikk/audit og flytt trafikken i et planlagt vindu. Lokale PVC-er flytter ikke innholdet automatisk ved å endre storageClass. Ved nytt cluster etableres kontrollplan på nytt; cluster-identitet og eventuelle secrets håndteres separat. Test tilbakeføringsplan og unngå to skrivende databaseprimærer. Utfør flyttingen før gammel vert avvikles, helst før kapasiteten er presset.

## Neste verifikasjon i #3408/#3424

- Avklar månedlig pris, lokasjon og nåværende OS/hypervisor, samt om serveren ellers ville blitt sagt opp.
- Les diskhelse, diskgrensesnitt, nettverksoppsett og faktisk ledig kapasitet etter autorisert tilgang; ingen nøkler skal postes i Favro.
- Velg VM- eller hybridmodell; spesifiser VPN-router og LB eksplisitt.
- Lag reviewbart bootstrap-/storageoppsett med parameteriserte grensesnitt, unik nodeidentitet og testet ruting.
- Mål app + database + hele driftsstakken; verifiser gjenstart, full disk, vertstap og ekstern restore i #3425.

Dette erstatter tidligere begrensning mot å undersøke konsolidering. Det er fortsatt et forslag, ikke en godkjent installasjonsplan.
