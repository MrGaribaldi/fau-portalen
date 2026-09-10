# Edge and DNS layer assessment

Card: #3443. Question from Erik, 9 September 2026: find a European alternative to Cloudflare,
and first establish whether one is needed at all, since Hetzner may already cover our use cases.

Status: assessment and recommendation. Nothing was purchased, registered or changed. No domain
exists yet (#3434), no DNS zone was created, and no account was opened anywhere.

## Summary

Cloudflare can be dropped without replacing it with another edge vendor. The functions people
buy Cloudflare for are either already solved inside the existing stack, or not needed by this
product at its current stage. Authoritative DNS is the only genuinely missing piece, and Hetzner
provides it for free — with one real limitation that needs a decision from Erik: Hetzner cannot
sign zones with DNSSEC.

Recommended: Hetzner DNS for the zone, no CDN, no WAF, no third-party DDoS layer for the MVP,
and a FAU-owned replacement for the upstream `dns_record` module. Decision needed on DNSSEC.

## Why this is not just a preference

Cloudflare is already wired into the infrastructure FAU inherits. `shared-modules/dns_record`
in infra-tools declares `cloudflare/cloudflare ~> 5.7.1` and creates `cloudflare_dns_record`
resources. Using upstream DNS handling unchanged means using Cloudflare and holding a Cloudflare
API token. Because the shared modules are consumed read-only from `/opt/infra-tools`, removing
Cloudflare means FAU owns a variant of that module.

Under the supplier ownership policy of 9 September 2026, DNS sits in the customer-facing
category and must be European, so this had to be resolved rather than inherited.

## What the existing stack already provides

Read from `/opt/infra-tools` on 9 September 2026:

| Function | Already provided by | Notes |
|---|---|---|
| TLS certificates | `shared-modules/operators/cert-manager` with a cluster issuer, `shared-modules/ingress`, `shared-modules/wildcard-certificate` | ACME inside the cluster; no external edge needed |
| HTTP routing, TLS termination | `shared-modules/operators/ingress-nginx` | nginx in-cluster |
| Public entry point | `hcloud_load_balancer` in `shared-modules/bootstrap/network/loadbalancer.tf` | fronts nginx |
| Authoritative DNS | `shared-modules/dns_record` — **Cloudflare** | the piece to replace |

Hetzner Cloud load balancers also support managed certificates, but adopting them would
duplicate the cert-manager path already in the stack. Keep certificate management in one place.

## Function-by-function: what does FAU actually need?

The cheapest way to satisfy a European requirement is not to need the function.

**Authoritative DNS — required.** Non-negotiable once a domain exists.

**CDN — not needed for the MVP.** The audience is Norwegian, effectively single-region, and the
application is mostly authenticated. The heavy payload is FAU documents and uploads, and the
storage decision of #3435 routes every download through the authorization layer with short-lived
signed URLs — content a shared CDN cannot cache anyway without breaking the tenant isolation the
same decision requires. What remains cacheable is the landing page (#3423) and a small static
bundle from the single application image, served from Falkenstein or Helsinki to Norwegian
users. That is a latency question of tens of milliseconds on a document tool, not a product
requirement. Revisit if a marketing campaign creates real burst traffic on public pages.

**WAF — not needed for the MVP.** No third-party WAF is required to run a Rust application
behind nginx. The relevant protections are application-level and already owned elsewhere:
tenant-scoped authorization (#3412), the storage rules of #3435, and safe Markdown rendering
(#3419). A WAF in front of an authenticated app mostly produces false positives on document
content.

**Bot and abuse protection — real, but not an edge purchase.** The genuine exposure is
self-service signup: magic-link abuse, email enumeration and automated FAU creation
(#3417, #3414, #3423, and the prefill links now specified on #3441). These need rate limiting,
verified email and careful error semantics in the application. An edge vendor would not solve
them, and buying one would not remove the need for them.

**DDoS — accepted for the MVP.** Hetzner states that DDoS protection is included with its
network; the precise scope of that protection is listed as an open item below. For a pre-revenue
service with no attacker incentive, network-level protection plus the ability to move an IP is a
reasonable position. A scrubbing service is a decision for the first paying customers, not now.

## Hetzner DNS, verified

Source: Hetzner DNS Console documentation, page last changed 2025-10-07, read 9 September 2026.

- **Free.** "We do not charge for our DNS zones."
- **Authoritative**, on three name servers: `hydrogen.ns.hetzner.com`, `oxygen.ns.hetzner.com`,
  `helium.ns.hetzner.com`.
- **Record types cover FAU's needs**, including the ones that matter for this project: A, AAAA,
  CNAME, MX, NS, TXT (SPF and DMARC), CAA, TLSA, SRV, PTR, DS, plus HTTPS and SVCB.
  TXT and CAA are what the transactional email work of #3410 and the ACME path need.
- **Works with a domain registered elsewhere**, so the registrar choice on #3434 stays open.
- **Terraform support exists but is community tier**: `germanbrew/hetznerdns` 3.5.0, published
  2025-10-24, roughly 78k downloads, source `github.com/germanbrew/terraform-provider-hetznerdns`.
  The older `timohirt/hetznerdns` is at 2.2.0 from 2022-10-15 and should be treated as
  unmaintained despite its higher download count.

### The one real limitation: no DNSSEC signing

Hetzner's own documentation states: "DNSSEC validation ... requires DS and DNSKey records. At the
moment, Hetzner only supports DS records." Hetzner can therefore publish a DS record delegating
trust to a zone signed elsewhere, but cannot sign a zone it hosts. Choosing Hetzner DNS means
FAU's zone is unsigned.

This is a decision for Erik, not something to assume either way:

1. **Accept no DNSSEC for now.** Common, and no worse than the majority of Norwegian sites.
   Weakens defence against DNS spoofing, which matters more than usual here because
   authentication is by magic link over email — an attacker who can spoof DNS is already in a
   strong position against both the web app and mail routing.
2. **Sign the zone elsewhere.** deSEC (Germany, free, DNSSEC-first, run with support from SSE) is
   the obvious European candidate, or the registrar's own DNS once #3434 settles. Cost is a
   second supplier to record on #3409 and no SLA on a free service.
3. **Revisit when it matters**, for example before real member data is in production.

My recommendation is option 1 for the MVP with option 2 reconsidered before production, because
DNSSEC without a signed parent delegation and monitoring is easy to misconfigure into an outage,
and there is no domain to sign yet.

## Terraform impact of dropping Cloudflare

- FAU needs its own `dns_record` equivalent. It is a thin module: one resource type, a zone
  reference, name, type, value, TTL.
- Community-tier provider risk is real but bounded. Pin the exact version, and keep the module
  small enough that swapping providers later is a contained change.
- Worth considering: **manage DNS outside Terraform.** FAU will have a handful of records —
  apex, www, the `notify` subdomain for email with its SPF, DKIM and DMARC TXT records, and CAA.
  Keeping them out of Terraform avoids a community provider in the critical path and keeps DNS
  answers from becoming state that a `destroy` can touch. The cost is that records are then not
  code-reviewed, which argues for Terraform if the record set grows.
- No stage-0 impact. Stage 0 has no DNS resources, so nothing already applied is affected.
- Cloudflare removal also removes a token from the credentials file that was never created.

## Cost

Hetzner DNS is free. Cloudflare's free tier is also free, so this is not a saving — it is a
compliance and dependency decision. Declining a CDN and WAF avoids new spend entirely; the
European CDN candidates that would otherwise be compared (Bunny.net, Gcore, Scaleway Edge
Services, OVHcloud) all start at real monthly cost for a function this product does not need yet.

## Open items, not verified

- Hetzner DNS zone and record limits. The documentation page for limits returned 404 on
  9 September 2026; the limits should be confirmed before committing, though FAU's record count
  will be trivially small.
- The precise scope of Hetzner's included DDoS protection, and what it explicitly does not
  cover. Their product pages did not yield a citable statement in this pass.
- Whether the registrar chosen on #3434 offers European-owned DNS with DNSSEC, which could make
  the DNSSEC question moot by consolidating registrar and DNS.
- Ownership and processing verification for Bunny.net, Gcore, Scaleway Edge Services, OVHcloud
  and deSEC. Deliberately not done, because the recommendation is to need none of them. If the
  CDN question reopens, that verification is the first step, and European branding must not be
  taken as proof of European ownership.

## Addendum: Domeneshop as registrar and DNS, 9 September 2026

Erik proposed domene.shop (Domeneshop), the registrar he already uses, and noted that with
static IPs the records are few enough to set up by hand. Both points hold, and the manual route
is now the recommended one — but checking it surfaced a second Cloudflare dependency that
matters more than the first.

### Domeneshop, verified

Source: api.domeneshop.no/docs and domene.shop, read 9 September 2026.

- Norwegian company, so European for the customer-facing category. Registrar and DNS from the
  same supplier, which is one supplier record on #3409 instead of two.
- DNS hosting with a documented REST API: list, create, update and delete records, HTTP Basic
  auth with a generated token and secret, plus client libraries and a maintained Certbot plugin
  for DNS-01 challenges.
- Record models documented in the API: A, AAAA, CNAME, MX, SRV, TXT. That covers the apex, www,
  and the SPF, DKIM and DMARC records the #3410 email work needs.
- Manual setup is realistic: the load balancer IPv4 and IPv6 are static, so the record set is
  roughly apex, www, and three or four TXT/CNAME records for email. Six to eight records that
  change almost never.

Two things to confirm in the control panel, which Erik can do faster than any research pass,
since he is already a customer: whether **CAA** records can be created (the API model list does
not include CAA, though the web panel may still allow it), and whether **DNSSEC** is offered for
the zone. If Domeneshop signs the zone, the Hetzner DNSSEC limitation disappears entirely and
the DNSSEC question is answered by consolidating registrar and DNS at Domeneshop.

### The finding that changes the recommendation: DNS-01 is Cloudflare-specific

`shared-modules/operators/cert-manager/cluster-issuer.tf` defines two ACME issuers. One uses an
**http01** solver. The other uses a **dns01** solver configured specifically for **Cloudflare**,
with a Cloudflare API token stored as a Kubernetes secret. So Cloudflare is wired into the
inherited stack in two places, not one: the `dns_record` Terraform module, and certificate
issuance.

This matters because DNS-01 is what wildcard certificates require, and
`shared-modules/wildcard-certificate` requests `*.<domain>` plus the apex. cert-manager has no
native Domeneshop solver, and Domeneshop publishes a Certbot plugin rather than a cert-manager
webhook, so DNS-01 against Domeneshop would mean writing and running a webhook solver.

Three ways out:

1. **Skip wildcard certificates; use HTTP-01 per hostname.** The `ingress` module already does
   this — `cert_http01` is created when no wildcard secret name is supplied. FAU needs few
   hostnames, and ADR-001 keeps the application on one origin with paths (`/`, `/app/`,
   `/api/v1/`) rather than per-tenant subdomains, so a wildcard buys nothing today. The `notify`
   subdomain is email-only and needs no certificate. **Recommended.**
2. Write a cert-manager webhook solver for the Domeneshop API. Real work, a new component to run
   and keep alive, for a capability point 1 shows we do not need.
3. Keep DNS somewhere with a native cert-manager DNS-01 solver. Reintroduces the supplier
   problem this card exists to remove.

The one thing that would overturn this: if FAU ever serves per-tenant subdomains
(`<fau-name>.example.no`), wildcard certificates and therefore DNS-01 come straight back. That
is a product decision, not an infrastructure one, and it is worth settling before the certificate
path is built, because retrofitting it means either a webhook solver or a DNS migration.

### Revised recommendation

Registrar and authoritative DNS at Domeneshop, records maintained by hand while they stay this
few, no Cloudflare in either place, no CDN, no WAF, and HTTP-01 certificates per hostname with no
wildcard. Removing the Cloudflare DNS-01 solver also removes the Cloudflare API token secret from
the cluster, which is one less credential to hold. Confirm CAA and DNSSEC in the Domeneshop panel,
and confirm that per-tenant subdomains are not planned.

## Resolved: DNSSEC, CAA and no subdomains — 9 September 2026

Erik approved the recommendation (skip wildcard certificates, issue per hostname) and answered
the two open items.

### DNSSEC: already handled by Domeneshop

Erik supplied a live zone transfer from an unused domain of his on Domeneshop. The zone is fully
signed, with no action required from us:

- `DNSKEY` records present: one KSK (flags 257) and two ZSKs (flags 256), all **algorithm 15**,
  which is Ed25519 — a modern choice, not the legacy RSA default.
- `RRSIG` covering every record set, including SOA, NS, A, AAAA, MX, TXT and the DNSKEY set
  itself, with roughly a 30-day signature validity window.
- `NSEC3PARAM` present, so authenticated denial of existence uses hashed NSEC3.
- Authoritative name servers `ns1`, `ns2`, `ns3.hyp.net`, with `hostmaster@domeneshop.no` as the
  SOA contact.

So DNSSEC is autoconfigured, and the Hetzner limitation that prompted the question is moot: FAU
does not need Hetzner DNS at all, and does not need to sign anything itself. Recorded as
verified from the zone rather than from marketing copy.

This also removes the last reason to consider deSEC or a second DNS supplier. One supplier,
Domeneshop, for both registrar and signed DNS.

### CAA, in plain terms

Erik asked what CAA means. A CAA record is a DNS record that lists which certificate authorities
are permitted to issue certificates for the domain. Every CA is required to check it before
issuing. With no CAA record — which is the case in the example zone, and is the common default —
**any** public CA may issue for the domain; with `0 issue "letsencrypt.org"` present, only Let's
Encrypt may, and another CA that is tricked into issuing must refuse.

It is a hardening measure, not a requirement, and nothing breaks without it:

- Recommended eventually, since FAU will use exactly one CA (Let's Encrypt, via cert-manager),
  which is the case where CAA costs nothing and closes a real gap.
- Worth more here than usual, because the zone is DNSSEC-signed: a signed CAA record cannot be
  stripped or spoofed in transit, so the restriction actually holds.
- Not a blocker for launch, and not something to chase now. Add it when the real domain exists
  (#3434), together with a `0 iodef "mailto:..."` reporting address if wanted. If Domeneshop's
  panel or API turns out not to offer CAA — the documented API record models do not list it —
  the consequence is simply that we go without, which is where most domains already are.

### No per-tenant subdomains: path structure instead

Erik: "no subdomains, they can use example.no/kommune/school-name instead."

This settles the certificate design permanently. With every FAU addressed by path on one origin,
there is never a `*.example.no` to cover, so per-hostname HTTP-01 is not a compromise for now but
the correct end state. It also fits ADR-001, which already keeps everything on one origin.

Three consequences that belong to other cards, recorded here so they are not discovered late:

1. **Root path segments become a namespace.** ADR-001 reserves `/app/`, `/api/v1/`, `/assets/`
   and `/health/`. If municipalities occupy the root (`/baerum/...`), the router needs an explicit
   reserved-word list, and the register on #3441 must never mint a slug that collides with one.
   A locale path prefix for public pages, left open on #3439, would add `/nb/` and `/nn/` to the
   same namespace — decide the order (`/nn/baerum/skole` versus `/baerum/nn/skole`) before either
   is built.
2. **Slugs need rules and stability.** Norwegian municipality and school names carry æ, ø and å,
   so slug generation must be defined rather than assumed, and the slug belongs in the database
   as a stable column rather than being derived from the display name at render time. Municipality
   mergers are a live problem in Norway, and schools are renamed and closed, so a renamed slug
   must keep working: old paths should redirect rather than 404, especially since #3441's outreach
   emails put these URLs in front of people who may click them months later.
3. **These paths are a public surface.** A page per school is a marketing asset as well as an
   address, which strengthens the case on #3439 for a locale path prefix on public pages, and
   means #3423 should treat these as indexable pages with their own titles rather than as an
   internal routing detail.

### Final recommendation, approved

Registrar and signed DNS at Domeneshop, records maintained by hand, no Cloudflare in either the
`dns_record` module or the cert-manager DNS-01 solver, no CDN, no WAF, HTTP-01 certificates per
hostname, no wildcard, and CAA deferred as optional hardening once the domain exists.
