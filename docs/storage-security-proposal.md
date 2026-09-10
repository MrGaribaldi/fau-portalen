# Private FAU file storage: agreed upload flow and open bucket decision

Research date: 7 September 2026.

Erik proposes originals/uploads in S3 with authoritative PostgreSQL ownership and object references, accessed through a validation layer. Shared buckets versus one per FAU remains open and should be evaluated for cost and security. This does not change the existing decision to store editable Markdown and change sets in PostgreSQL.

## Verified supplier facts

- Hetzner charges its base fee per account independently of bucket count. Additional empty buckets do not incur separate base fees. Aggregate storage/traffic still count. Limits documented: 100 buckets and 200 S3 credentials across projects. Tiny objects have a 64 kB minimum billable size. Source: https://docs.hetzner.com/storage/object-storage/overview/
- Keys have read/write access to existing and new buckets in their project by default. Hetzner documents explicit blocking/allowlisting and cross-project key policies. Source: https://docs.hetzner.com/storage/object-storage/faq/s3-credentials/
- Private bucket labels do not guarantee all objects are private: object ACLs and policies matter. Presigned URLs are usable by anyone possessing them until expiry. Source: https://docs.hetzner.com/storage/object-storage/faq/buckets-objects/

## Recommended design to review

- Shared private document bucket with opaque FAU prefixes is a scalable candidate; per-FAU buckets provide useful policy separation only with correctly restricted credentials. Confirm quota expansion before treating per-FAU buckets as a national-scale design. No topology selected yet.
- PostgreSQL stores file ID, tenant ID, storage location/bucket, immutable object key, original filename, size/hash/type and lifecycle status. Store neither a permanent public URL nor a presigned URL as authority.
- Client requests a file ID. Backend validates session, current membership/role and tenant context for every operation, resolves a tenant-scoped metadata row, verifies its object reference belongs to that tenant, and only then accesses storage. UI filtering and unguessable IDs are insufficient authorization.
- Scope uploads, downloads, previews, replacements, deletes and future exports/version access identically. Server generates object keys. Missing or inconsistent context denies access. Grace-only admins must not gain document access by virtue of handover permissions.
- Agreed upload flow: backend verifies membership in the active FAU, creates a pending file record and generates the exact destination key; browser uploads directly to private S3 using a short-lived signed upload URL. No browser S3 credentials and no client-selected bucket, tenant prefix or existing object key. Backend independently checks stored size and validates content before making the file available. Download access continues through the authorization layer; backend-proxied downloads remain the current proposal. Cache headers and cache keys must prevent cross-tenant delivery.
- Signed upload URLs are temporary bearer permissions, not assumed single-use. Verify Hetzner-supported limits, replay/overwrite protection and abandoned-upload cleanup. Finalization must ensure that the validated bytes cannot subsequently be replaced using an unexpired upload URL; choose and test the enforcement mechanism before implementation is accepted.
- Keep document storage separate from public static assets and backups. Separate provisioning credentials from app data-access credentials; runtime should not change bucket policy/ACL or manage backup storage. Validate actual Hetzner policy behavior, not AWS feature parity assumptions.
- Require automated negative tests: FAU A cannot list/read/write/overwrite/delete FAU B files; wrong metadata/key mapping fails; unauthenticated direct S3 reads fail; membership revocation blocks access; context switching and caches cannot leak files. Test policy drift/public object settings as well as application paths.
- Broad backend credentials remain a blast-radius risk: bucket count alone cannot protect against compromised code able to select any tenant's credentials. Evaluate an independently scoped storage access layer and database enforcement without assuming either is already implemented.

## Acceptance before choosing

Document shared vs per-FAU cost/quota/operations tradeoffs, prove the proposed policy restrictions in an isolated authorized test environment, specify application authorization and adversarial tests, and obtain a recorded bucket architecture decision. Direct signed uploads with backend authorization and validation are agreed; bucket topology and enforcement mechanisms remain open. This card does not provision buckets.
