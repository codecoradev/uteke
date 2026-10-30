# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| Latest release | ✅ |
| Previous release | ✅ (critical only) |
| Older versions | ❌ |

## Reporting a Vulnerability

If you discover a security vulnerability in Uteke, please report it responsibly.

**Do NOT** open a public issue for security vulnerabilities.

### How to Report

Use [GitHub Private Vulnerability Reporting](https://github.com/codecoradev/uteke/security/advisories/new) — this ensures the report is confidential and only visible to maintainers.

Include as much detail as possible:

- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

### What to Expect

- **Acknowledgment** within 48 hours
- **Initial assessment** within 5 business days
- **Fix timeline** depends on severity:
  - Critical: 7 days
  - High: 14 days
  - Medium: 30 days
  - Low: next minor release

### Security in the Development Process

Uteke uses multiple automated security checks on every PR:

- **Cargo Audit** — dependency vulnerability scanning
- **Trivy FS Scan** — filesystem security scanning
- **GitGuardian** — secret leak detection

These are enforced via CI and block merge on findings.

## Verifying a release

Since v0.21.0, `checksums-sha256.txt` of every GitHub release
is signed with [cosign](https://docs.sigstore.dev/) keyless signing from the
`Release` workflow (#1327). The signature bundle is published next to it as
`checksums-sha256.txt.bundle`. There is no signing key to trust: the certificate
inside the bundle is bound to this repository's workflow identity.

```bash
VER=vX.Y.Z   # the release tag
curl -fsSLO "https://github.com/codecoradev/uteke/releases/download/$VER/checksums-sha256.txt"
curl -fsSLO "https://github.com/codecoradev/uteke/releases/download/$VER/checksums-sha256.txt.bundle"

cosign verify-blob \
  --bundle checksums-sha256.txt.bundle \
  --certificate-identity "https://github.com/codecoradev/uteke/.github/workflows/release.yml@refs/tags/$VER" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
  checksums-sha256.txt

# then check the archive you downloaded against the verified file
sha256sum --check --ignore-missing checksums-sha256.txt
```

### What `uteke upgrade` verifies

`uteke upgrade` checks that bundle itself, offline, before it trusts any hash in
`checksums-sha256.txt` and then checks the archive against that file (#1610). It
needs nothing beyond the downloads: the Sigstore roots (the Fulcio root and
intermediate, the Rekor log key) are compiled into the binary. It verifies:

- the signature over `checksums-sha256.txt`, made by the key in the bundle's
  certificate;
- that the certificate chains to the embedded Fulcio root and is a code-signing
  certificate, not a CA;
- that its identity is this repository's `release.yml` workflow at exactly the
  tag being installed, issued to GitHub Actions (so one release's files cannot be
  served as another's);
- that the Rekor entry's signed timestamp verifies against the embedded log key
  and lies inside the certificate's 10-minute validity, and that the entry names
  the same file, signature and certificate.

`uteke upgrade` installs only stable, published releases (`vX.Y.Z`), refuses to
downgrade, and refuses any release older than the first one it can verify
(v0.22.1): a pre-release or build-metadata tag is refused too. A missing or
invalid bundle is a hard failure, never a silent downgrade to checksum-only, and
there is no fallback to an older scheme. Setting `UTEKE_UPGRADE_SKIP_CHECKSUM=1`
skips both checks (not recommended).

Builds before v0.22.1 (they verified an offline minisign signature, now retired)
cannot verify a release signed this way. Reinstall once from a package manager
(`brew upgrade`, `cargo install uteke-cli`) or a manual download; from v0.22.1 on
`uteke upgrade` works as before.

The trust anchor is the workflow identity, so the control that matters is who can
create a `v*` tag: the `release-tags-protected` repository ruleset restricts tag
creation, update and deletion to the repository Admin role. A compromised
maintainer account or a new repository admin can therefore publish a release
that `uteke upgrade` accepts, the same trust the GitHub release itself needs.
The embedded Sigstore roots can only be replaced by a uteke release, because an
installed binary cannot learn a new root by itself.

The signature is checked by `uteke upgrade` ONLY. `install.sh`,
`install.ps1` and Homebrew do not verify it: Homebrew relies on the SHA256s in
the formula, and for the scripts you should verify manually as described above.
