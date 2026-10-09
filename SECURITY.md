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

Since the release after v0.20.0, `checksums-sha256.txt` of every GitHub release
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

`uteke upgrade` does not verify this signature yet; it still checks the archive
against the unsigned `checksums-sha256.txt` of the same release (corruption
detection only).
