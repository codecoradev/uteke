# uteke-privacy

The secret detector of [uteke](https://github.com/codecoradev/uteke), as a small crate with no IO and no storage. It finds common secret shapes in text and masks them in a way that keeps the shape, so the text stays readable and the output stays stable.

It depends on `regex` only. `uteke-core` re-exports it, and `tole` uses it on its egress paths, so both projects share one ruleset.

## Usage

```rust
use uteke_privacy::{Policy, redact};

// A made-up token, assembled at run time so this file holds no scanner bait.
let token = format!("{}{}", "ghp_", "aB3dE5gH7jK9mN1pQ3rS5tU7vW9xY1zA3bC5");
let out = redact(&format!("token {token}"), &Policy::default());
assert_eq!(out.text, "token ghp_[REDACTED:github-token]");
assert_eq!(out.total(), 1);
// Counts name the kind, never the value.
assert_eq!(out.counts[0].kind, "github-token");
```

`redact` returns `Redacted { text, counts }`. It is idempotent: redacting the output again changes nothing.

## What it detects

AWS access keys, GitHub tokens, npm tokens, Slack tokens, Google API keys, `sk-` style API keys, bearer headers, JWTs, PEM private key blocks, passwords inside URLs, and `NAME=value` assignments whose name looks secret and whose value is long and random enough (hashes, UUIDs, paths, placeholders and the like are left alone).

Masked shapes keep the prefix: `ghp_[REDACTED:github-token]`, `Bearer [REDACTED:bearer-token]`, `API_KEY=[REDACTED:env-secret]`.

Precision comes first. A rule is only added with a false-positive test.

## Allowlist

`Policy::from_patterns(&["^dev-".to_string()])` builds a policy whose allowlist (regexes) keeps any matching value as it is. An invalid regex is a `PolicyError`.

## License

Apache-2.0
