# Keyless verification for `uteke upgrade` (cosign / Sigstore)

Status: design, 2026-10-30. Decide before implementing. Replaces the offline minisign step of #1327.

## Why

`uteke upgrade` replaces the running binary with a file from a GitHub release. Since #1327 it
verifies a minisign signature of `checksums-sha256.txt`, made on the maintainer's machine with an
offline key (`scripts/sign-release.sh`). That gate has three costs:

- one manual step per release, with a password;
- a lost password means unsigned releases (it happened on 2026-10-30, v0.22.0 is unsigned);
- rotating the key breaks every installed binary, because binaries only trust the keys compiled
  into them (`RELEASE_PUBLIC_KEYS`).

The Release workflow already signs the same file with cosign, keyless, on every release
(`checksums-sha256.txt.bundle`). `sign-release.sh` even verifies that bundle before it signs. The
proposal is to let `uteke upgrade` verify that bundle itself and retire the offline step.

## What the bundle is (measured on v0.22.0)

Legacy cosign bundle, JSON with three keys:

| key | content |
|---|---|
| `base64Signature` | ECDSA P-256 signature, DER (71 bytes), over the SHA-256 of `checksums-sha256.txt` |
| `cert` | base64 of a PEM Fulcio leaf certificate |
| `rekorBundle` | `SignedEntryTimestamp` + `Payload {body, integratedTime, logIndex, logID}` |

The leaf certificate: issuer `O=sigstore.dev, CN=sigstore-intermediate`; valid for 10 minutes
(03:47:34 to 03:57:34 on 2026-10-30); key usage `digitalSignature`; extended key usage `Code Signing`;
subject alternative name URI
`https://github.com/codecoradev/uteke/.github/workflows/release.yml@refs/tags/v0.22.0`; Fulcio OIDC
extensions under `1.3.6.1.4.1.57264.1.*` (issuer `https://token.actions.githubusercontent.com`).
The same bundle exists on v0.21.0 and later.

## Verification (offline, pure Rust, fail closed)

Given the tag being installed, `checksums-sha256.txt` and its bundle:

1. **Signature.** SHA-256 the file; verify the ECDSA P-256 signature with the leaf's public key.
2. **Chain.** The leaf chains to an embedded Sigstore public-good root through the intermediate,
   with `Code Signing` extended key usage, checked at the time of step 4 (not at "now": the
   certificate lives 10 minutes).
3. **Identity.** The SAN URI equals
   `https://github.com/codecoradev/uteke/.github/workflows/release.yml@refs/tags/<tag>` for exactly the
   tag being installed (this is the replay defence the minisign trusted comment gave: an older
   release's files cannot be passed off as a newer one), and the OIDC issuer extension equals
   `https://token.actions.githubusercontent.com`.
4. **Time.** `integratedTime` of the Rekor entry lies inside the certificate validity, and the
   `SignedEntryTimestamp` verifies against the embedded Rekor public key. This proves the signature
   was made while the certificate was valid; without it a leaked ephemeral key could sign later.
5. **Entry consistency.** The decoded Rekor `body` (a `hashedrekord`) names the same artifact hash,
   signature and certificate.
6. Existing limits stay: size caps, tag format restriction, `UTEKE_UPGRADE_SKIP_CHECKSUM` behaviour.

No network call is needed beyond the downloads that exist today (no Rekor lookup, no TUF).

### Dependencies

`ring` 0.17 and `rustls-webpki` 0.103 are already in the tree (through rustls). webpki verifies a
chain against caller-supplied trust anchors at a caller-supplied time, with an extended key usage
requirement, and verifies a signature with the end-entity key; `ring` verifies ECDSA P-256. Not
covered: reading the SAN URI and the Fulcio OID extensions (webpki does not expose them). Options:
a minimal DER walk (about 100 lines, strict, unit-tested) or `x509-parser` (pure Rust, one new
dependency). The `sigstore` crate is not proposed (large, async, brings its own TLS and TUF stack).

## Trust roots

Embedded in the binary: the Fulcio root and intermediate, and the Rekor public key, as lists (so a
new root can be added next to the old). They rotate rarely, but when they do an installed binary
cannot learn the new one: the same limit as `RELEASE_PUBLIC_KEYS` today. Mitigations: a scheduled
workflow compares the embedded roots with the current Sigstore TUF root and opens an issue well
before an expiry; a release that adds the new root ships before the old one expires.

## Rollout

1. Verifier plus fixtures and tests (below). The scheme is chosen by the version of the tag being
   installed, never by which files the release happens to carry: a tag from v0.22.1 on is accepted
   only with a valid cosign bundle, and a missing or invalid bundle is a refusal (no fallback to
   minisign, so a missing bundle cannot downgrade the check). `uteke upgrade` only installs the latest
   release, and the latest release is v0.22.1 or newer once this ships, so the minisign path has no
   caller left and `minisign-verify` is removed from the dependencies in the same change.
2. Ship in 0.22.1. Retire `scripts/sign-release.sh`, its test and the signature reminder workflow;
   update AGENTS.md (release step 6), SECURITY.md ("Verifying a release") and the CHANGELOG.
3. Installed binaries (0.21.x and 0.22.0) cannot verify the new scheme: they reinstall once with
   `brew upgrade`, `cargo install uteke-cli` or a manual download. From 0.22.1 on `uteke upgrade` works
   as before. Say so in the release notes and in SECURITY.md.

## Tests

Fixtures from a real release (bundle, checksums, certificate chain). Positive: v0.22.0. Negative, each
refused with its own message: file changed by one byte; bundle of another release; identity of
another tag; identity of another workflow or repository; issuer other than GitHub Actions;
`integratedTime` outside the certificate validity; altered `SignedEntryTimestamp`; altered Rekor
body; certificate without Code Signing; chain to an unknown root; truncated or oversized bundle.

## What is lost, and what replaces it

The offline key was an independent gate: even a fully compromised GitHub could not produce a valid
update. Keyless verification trusts the workflow identity, so whoever can create a `v*` tag can produce
an accepted release: a modified `release.yml` on a new tag gets a valid certificate for that tag.

**The one control that closes this path is a protected `v*` tag** (a repository ruleset: creation
restricted to the maintainer, no update, no deletion, and an empty bypass list). Without it, nothing
below matters.

These are defence in depth and do NOT stop a modified workflow on a new tag, because the attacker's
copy of the file can drop them; do not count them as equivalent to the tag rule:

- a GitHub Environment with a required reviewer: it only applies if the workflow declares
  `environment: release`, which a modified copy can omit. It does protect against an honest workflow
  being run by a stolen token that cannot edit it;
- CODEOWNERS and branch protection on `release.yml`: they guard branches, not the commit a tag points
  to (the existing "Verify tag is on main" job is inside the workflow and can be removed the same way).

The residual risk is therefore: a compromised maintainer account or a repository admin who can create
tags. That is the same trust the GitHub release itself already needs, and it is smaller than the
alternative of a signing key stored as a CI secret, which can be exfiltrated and used anywhere, at any
time.

## Open questions

- Accept the legacy bundle only, or also switch the workflow to the new protobuf bundle
  (`cosign sign-blob --new-bundle-format`) and accept both?
- Environment reviewer on the release jobs: yes or no (defence in depth only, it adds a click per release).
- Who may create `v*` tags, and who is on the ruleset bypass list (it must be empty).
- DER walk or `x509-parser`.
- Alternative to all of this: remove `uteke upgrade` and point to the package managers (simplest,
  loses self-update for manual installs).
