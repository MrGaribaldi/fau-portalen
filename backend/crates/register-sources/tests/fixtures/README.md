# Recorded source fixtures (#3441)

Recorded on 24 September 2026 from the live public APIs, and committed so that no test ever calls
the network (docs/school-register-design.md §10). Everything here is open public data, except as
noted under Brreg.

| Directory | Source | Licence | Recorded from |
|---|---|---|---|
| `nsr/` | Udir, Nasjonalt skoleregister | NLOD | `https://data-nsr.udir.no/v4/enheter?sidenummer=1&antallperside=5`, `/v4/enheter/kommune/3201`, `/v4/enhet/{orgnr}` |
| `kartverket/` | Kartverket, Administrative enheter | CC BY 4.0 | `https://api.kartverket.no/kommuneinfo/v1/fylkerkommuner` |
| `ssb/` | SSB Klass 131 | CC BY 4.0 | `https://data.ssb.no/api/klass/v1/classifications/131/changes?from=2023-12-01&to=2024-01-31`, and `from=2025-12-01&to=2026-01-31` |
| `brreg/` | Brønnøysundregistrene, Enhetsregisteret | NLOD | `https://data.brreg.no/enhetsregisteret/api/enheter/{orgnr}`, see below |

## The NSR units, and why each one is here

The detail records are:

| Orgnr | School | Case |
|---|---|---|
| 974552124 | Hosle skole, 3201 Bærum | ordinary public school |
| 990672938 | Norges Toppidrettsgymnas ungdomsskole Bærum AS | private |
| 998516897 | Lerberg skole og kompetansesenter | combined: 85.201 at priority 1, 85.310 at priority 2 |
| 998666783 | Signo Grunn- og videregående skole AS | special school (85.202) |
| 999038182 | Lørenskog voksenopplæring | adult education |
| 986779795 | Wang Fredrikstad AS | upper secondary as its primary NACE code |
| U90099017 | Den norske skole - Gran Canaria | abroad (2599) |
| 974795655 | Longyearbyen skole grunnskole | Svalbard (2100) |
| 998245508 | Kjølsdalen montessoriskule SA | Nynorsk |
| 998670799 | Halsa barne- og ungdomsskole | no website |
| 974554682 | Holtålen kommune Haltdalen oppvekstsenter avd skole | municipality prefix in the name |
| 975270920 / 933181995 | Stange ungdomsskole, old and new | re-registration pair |

## Brreg: redacted, on purpose

Brreg records of FAU-er carry personal data: `c/o <parent>` address lines, and `epostadresse`,
`mobil` and `telefon` fields. `brreg/enheter-sample.json` is real records with **every
`epostadresse`, `mobil` and `telefon` removed**. Two records are altered and say so in
`_fixture_note`:
- 913591100 has an invented `c/o` line, street, e-mail and mobile, in place of a real person's;
- 999999999 is synthetic: a copy of the Hosle FAU with a `c/o Hosle skole` line.

The file is an array, like the bulk file `enheter/lastned` once decompressed. Tests gzip it in
memory. Never commit an unredacted Brreg record.
