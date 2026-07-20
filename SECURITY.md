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

1. Open a new issue with the title prefix `[SECURITY]`
2. Use the label `security`
3. Include as much detail as possible:
   - Description of the vulnerability
   - Steps to reproduce
   - Potential impact
   - Suggested fix (if any)
4. The issue will be restricted to maintainers only

Alternatively, contact the maintainer directly via [GitHub Security Advisories](https://github.com/codecoradev/uteke/security/advisories/new).

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
