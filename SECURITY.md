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

### minisign signature

The release maintainer also signs `checksums-sha256.txt` offline with a
[minisign](https://jedisct1.github.io/minisign/) key that never leaves their
machine (CI holds no signing key). The signature is published as
`checksums-sha256.txt.minisig`; its trusted comment is `uteke <tag>`, binding it
to one release. The public key is `scripts/uteke-release.pub`, key id
`96924B76E31B048B`.

```bash
curl -fsSLO "https://github.com/codecoradev/uteke/releases/download/$VER/checksums-sha256.txt.minisig"

minisign -Vm checksums-sha256.txt \
  -P RWSLBBvjdkuSlvaQ/yoyoMlpQAOLTt5jACZGTgSOCKb+/1JoqhjOmT8G
# the output must show: Trusted comment: uteke vX.Y.Z  (your $VER)
```

Since the release that ships this change, `uteke upgrade` verifies this
minisign signature over `checksums-sha256.txt` before trusting any hash in it,
then checks the archive against that file. The signature is bound to the tag
(the trusted comment must be `uteke <tag>`), so one release's files cannot be
served as another's, and `uteke upgrade` refuses to downgrade to an older
release. A missing or invalid signature is a hard failure, never a silent
downgrade to checksum-only. Setting `UTEKE_UPGRADE_SKIP_CHECKSUM=1` skips both
checks (not recommended). Releases published before the signature existed
cannot be installed with `uteke upgrade` from a build that verifies it.

The minisign signature is checked by `uteke upgrade` ONLY. `install.sh`,
`install.ps1` and Homebrew do not verify it: Homebrew relies on the SHA256s in
the formula, and for the scripts you should verify manually as described above.
