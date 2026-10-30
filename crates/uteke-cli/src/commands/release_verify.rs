//! Keyless (Sigstore) verification of a release's `checksums-sha256.txt` (#1610).
//!
//! The Release workflow signs the checksums file with `cosign sign-blob` and
//! publishes the legacy bundle (`checksums-sha256.txt.bundle`): the signature,
//! the short-lived Fulcio certificate and the Rekor entry. This module checks
//! all of it offline, with no network call and no new trust beyond the roots
//! embedded in the binary (`trust/`):
//!
//! 1. the signature is a valid ECDSA P-256 signature of the file, made by the
//!    key in the leaf certificate;
//! 2. the leaf chains to an embedded Fulcio root through an embedded
//!    intermediate, and is a code-signing leaf (not a CA);
//! 3. the leaf identity is this repository's release workflow at exactly the
//!    tag being installed, issued to GitHub Actions (the replay defence: an
//!    older release's files cannot pass as a newer one);
//! 4. the Rekor `SignedEntryTimestamp` verifies against the embedded Rekor key
//!    and its `integratedTime` lies inside every certificate's validity, so the
//!    signature was made while the 10-minute certificate was valid;
//! 5. the Rekor entry names the same file hash, signature and certificate.
//!
//! Everything fails closed: a missing or malformed part is a refusal.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use ring::signature::{
    ECDSA_P256_SHA256_ASN1, ECDSA_P384_SHA384_ASN1, UnparsedPublicKey, VerificationAlgorithm,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use x509_parser::certificate::X509Certificate;
use x509_parser::der_parser::oid::Oid;
use x509_parser::extensions::GeneralName;
use x509_parser::prelude::FromDer;
use x509_parser::time::ASN1Time;
use x509_parser::x509::SubjectPublicKeyInfo;

/// Fulcio root of the Sigstore public-good instance (self-signed, ECDSA P-384).
const FULCIO_ROOT: &[u8] = include_bytes!("../../trust/fulcio-root.der");
/// Fulcio intermediate that issues the release leaves, signed by the root.
const FULCIO_INTERMEDIATE: &[u8] = include_bytes!("../../trust/fulcio-intermediate.der");
/// Rekor (rekor.sigstore.dev) log public key, SPKI DER, ECDSA P-256.
const REKOR_KEY: &[u8] = include_bytes!("../../trust/rekor-public-key.der");

/// Where the workflow identity is anchored: this repository's release workflow.
const WORKFLOW_IDENTITY_PREFIX: &str =
    "https://github.com/codecoradev/uteke/.github/workflows/release.yml@refs/tags/";
/// The OIDC issuer of GitHub Actions, as Fulcio records it in the leaf.
const GITHUB_ACTIONS_ISSUER: &str = "https://token.actions.githubusercontent.com";

/// Fulcio OIDC issuer extension, legacy form (raw string): 1.3.6.1.4.1.57264.1.1.
const OID_ISSUER_V1: &[u64] = &[1, 3, 6, 1, 4, 1, 57264, 1, 1];
/// Fulcio OIDC issuer extension, current form (DER UTF8String): ...57264.1.8.
const OID_ISSUER_V2: &[u64] = &[1, 3, 6, 1, 4, 1, 57264, 1, 8];
/// ecdsa-with-SHA256 and ecdsa-with-SHA384 signature algorithms.
const OID_ECDSA_SHA256: &[u64] = &[1, 2, 840, 10045, 4, 3, 2];
const OID_ECDSA_SHA384: &[u64] = &[1, 2, 840, 10045, 4, 3, 3];

/// The roots a bundle is checked against. Lists, so a new root can ship next to
/// the old one before the old one expires.
pub(crate) struct TrustRoots<'a> {
    pub roots: &'a [&'a [u8]],
    pub intermediates: &'a [&'a [u8]],
    pub rekor_keys: &'a [&'a [u8]],
}

/// The Sigstore public-good roots embedded in this binary.
pub(crate) const PUBLIC_GOOD: TrustRoots<'static> = TrustRoots {
    roots: &[FULCIO_ROOT],
    intermediates: &[FULCIO_INTERMEDIATE],
    rekor_keys: &[REKOR_KEY],
};

#[derive(Deserialize)]
struct Bundle {
    #[serde(rename = "base64Signature")]
    signature: String,
    cert: String,
    #[serde(rename = "rekorBundle")]
    rekor: RekorBundle,
}

#[derive(Deserialize)]
struct RekorBundle {
    #[serde(rename = "SignedEntryTimestamp")]
    signed_entry_timestamp: String,
    #[serde(rename = "Payload")]
    payload: RekorPayload,
}

#[derive(Deserialize)]
struct RekorPayload {
    body: String,
    #[serde(rename = "integratedTime")]
    integrated_time: i64,
    #[serde(rename = "logIndex")]
    log_index: i64,
    #[serde(rename = "logID")]
    log_id: String,
}

fn oid(parts: &[u64]) -> Result<Oid<'static>, String> {
    Oid::from(parts).map_err(|_| "internal: invalid OID constant".to_string())
}

fn b64(what: &str, text: &str) -> Result<Vec<u8>, String> {
    B64.decode(text.trim())
        .map_err(|e| format!("{what} is not valid base64 ({e})"))
}

fn parse_cert<'a>(what: &str, der: &'a [u8]) -> Result<X509Certificate<'a>, String> {
    let (rest, cert) = X509Certificate::from_der(der)
        .map_err(|e| format!("{what} is not a valid X.509 certificate ({e})"))?;
    if !rest.is_empty() {
        return Err(format!("{what} has trailing data after the certificate"));
    }
    Ok(cert)
}

/// Decode the PEM certificate a bundle carries (base64 of a PEM block) to DER.
fn pem_to_der(pem_text: &[u8]) -> Result<Vec<u8>, String> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(pem_text)
        .map_err(|e| format!("the bundle certificate is not valid PEM ({e})"))?;
    if pem.label != "CERTIFICATE" {
        return Err("the bundle certificate is not a CERTIFICATE PEM block".to_string());
    }
    Ok(pem.contents)
}

/// Verify `cert` was signed by `issuer` (ECDSA, P-256/SHA-256 or P-384/SHA-384).
fn check_issued_by(
    what: &str,
    cert: &X509Certificate<'_>,
    issuer: &X509Certificate<'_>,
) -> Result<(), String> {
    let alg: &dyn VerificationAlgorithm = {
        let sig_oid = &cert.signature_algorithm.algorithm;
        if *sig_oid == oid(OID_ECDSA_SHA256)? {
            &ECDSA_P256_SHA256_ASN1
        } else if *sig_oid == oid(OID_ECDSA_SHA384)? {
            &ECDSA_P384_SHA384_ASN1
        } else {
            return Err(format!("{what} uses an unsupported signature algorithm"));
        }
    };
    let key = UnparsedPublicKey::new(alg, issuer.public_key().subject_public_key.data.as_ref());
    key.verify(
        cert.tbs_certificate.as_ref(),
        cert.signature_value.data.as_ref(),
    )
    .map_err(|_| format!("{what} is not signed by the trusted issuer"))
}

fn check_ca(what: &str, cert: &X509Certificate<'_>) -> Result<(), String> {
    let bc = cert
        .basic_constraints()
        .map_err(|e| format!("{what} has bad basic constraints ({e})"))?;
    match bc {
        Some(bc) if bc.value.ca => Ok(()),
        _ => Err(format!("{what} is not a CA certificate")),
    }
}

/// The chain `leaf <- intermediate <- root`, each valid at `at`.
fn check_chain(
    leaf: &X509Certificate<'_>,
    trust: &TrustRoots<'_>,
    at: ASN1Time,
) -> Result<(), String> {
    for inter_der in trust.intermediates {
        let inter = parse_cert("embedded Fulcio intermediate", inter_der)?;
        if leaf.issuer() != inter.subject() || check_issued_by("leaf", leaf, &inter).is_err() {
            continue;
        }
        check_ca("the Fulcio intermediate", &inter)?;
        for root_der in trust.roots {
            let root = parse_cert("embedded Fulcio root", root_der)?;
            if inter.issuer() != root.subject()
                || check_issued_by("intermediate", &inter, &root).is_err()
            {
                continue;
            }
            check_ca("the Fulcio root", &root)?;
            for (name, cert) in [("leaf", leaf), ("intermediate", &inter), ("root", &root)] {
                if !cert.validity().is_valid_at(at) {
                    return Err(format!(
                        "the {name} certificate was not valid when the signature was logged"
                    ));
                }
            }
            return Ok(());
        }
    }
    Err("the certificate does not chain to a trusted Sigstore root".to_string())
}

/// The leaf must be a code-signing end-entity certificate.
fn check_leaf_usage(leaf: &X509Certificate<'_>) -> Result<(), String> {
    let bc = leaf
        .basic_constraints()
        .map_err(|e| format!("the signing certificate has bad basic constraints ({e})"))?;
    if bc.is_some_and(|b| b.value.ca) {
        return Err("the signing certificate is a CA, not a code-signing leaf".to_string());
    }
    let ku = leaf
        .key_usage()
        .map_err(|e| format!("the signing certificate has bad key usage ({e})"))?;
    if !ku.is_some_and(|k| k.value.digital_signature()) {
        return Err("the signing certificate is not allowed to make signatures".to_string());
    }
    let eku = leaf
        .extended_key_usage()
        .map_err(|e| format!("the signing certificate has bad extended key usage ({e})"))?;
    if !eku.is_some_and(|k| k.value.code_signing) {
        return Err("the signing certificate is not a code-signing certificate".to_string());
    }
    Ok(())
}

/// The leaf identity: the release workflow of this repository at exactly `tag`,
/// issued to GitHub Actions.
fn check_identity(leaf: &X509Certificate<'_>, tag: &str) -> Result<(), String> {
    let expected = format!("{WORKFLOW_IDENTITY_PREFIX}{tag}");
    let san = leaf
        .subject_alternative_name()
        .map_err(|e| format!("the signing certificate has a bad identity ({e})"))?
        .ok_or_else(|| "the signing certificate carries no identity".to_string())?;
    match san.value.general_names.as_slice() {
        [GeneralName::URI(uri)] if *uri == expected => {}
        [GeneralName::URI(uri)] => {
            return Err(format!(
                "the signature was made for a different workflow or release \
                 (identity \"{uri}\", expected \"{expected}\")"
            ));
        }
        _ => return Err("the signing certificate has an unexpected identity form".to_string()),
    }

    let mut issuers = Vec::new();
    if let Some(ext) = leaf
        .get_extension_unique(&oid(OID_ISSUER_V1)?)
        .map_err(|e| format!("the signing certificate has bad extensions ({e})"))?
    {
        issuers.push(ext.value.to_vec());
    }
    if let Some(ext) = leaf
        .get_extension_unique(&oid(OID_ISSUER_V2)?)
        .map_err(|e| format!("the signing certificate has bad extensions ({e})"))?
    {
        // DER UTF8String: tag 0x0c, short-form length, content.
        match ext.value {
            [0x0c, len, rest @ ..] if usize::from(*len) == rest.len() && *len < 0x80 => {
                issuers.push(rest.to_vec());
            }
            _ => return Err("the signing certificate has a malformed issuer".to_string()),
        }
    }
    if issuers.is_empty()
        || issuers
            .iter()
            .any(|i| i != GITHUB_ACTIONS_ISSUER.as_bytes())
    {
        return Err("the signing certificate was not issued to GitHub Actions".to_string());
    }
    Ok(())
}

/// Verify the Rekor `SignedEntryTimestamp` and return the logged time.
///
/// The signed message is the canonical JSON of the payload (keys sorted, no
/// whitespace), the form cosign's legacy bundle uses.
fn check_rekor_timestamp(rekor: &RekorBundle, trust: &TrustRoots<'_>) -> Result<i64, String> {
    let payload = &rekor.payload;
    let canonical = format!(
        "{{\"body\":{},\"integratedTime\":{},\"logID\":{},\"logIndex\":{}}}",
        serde_json::to_string(&payload.body).map_err(|e| e.to_string())?,
        payload.integrated_time,
        serde_json::to_string(&payload.log_id).map_err(|e| e.to_string())?,
        payload.log_index,
    );
    let set = b64("the Rekor timestamp", &rekor.signed_entry_timestamp)?;
    for key_der in trust.rekor_keys {
        if hex(&Sha256::digest(key_der)) != payload.log_id {
            continue;
        }
        let (_, spki) = SubjectPublicKeyInfo::from_der(key_der)
            .map_err(|e| format!("embedded Rekor key is invalid ({e})"))?;
        let key = UnparsedPublicKey::new(
            &ECDSA_P256_SHA256_ASN1,
            spki.subject_public_key.data.as_ref(),
        );
        return key
            .verify(canonical.as_bytes(), &set)
            .map(|()| payload.integrated_time)
            .map_err(|_| {
                "the Rekor timestamp does not verify (altered or forged log entry)".into()
            });
    }
    Err("the Rekor entry is from a log that is not trusted".to_string())
}

/// The logged `hashedrekord` must describe this very file, signature and cert.
fn check_rekor_body(
    payload: &RekorPayload,
    checksums: &[u8],
    signature: &[u8],
    leaf_der: &[u8],
) -> Result<(), String> {
    let body = b64("the Rekor entry body", &payload.body)?;
    let doc: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| format!("the Rekor entry body is not valid JSON ({e})"))?;
    let text = |path: &[&str]| -> Option<&str> {
        path.iter()
            .try_fold(&doc, |v, k| v.get(k))
            .and_then(serde_json::Value::as_str)
    };
    if text(&["kind"]) != Some("hashedrekord") {
        return Err("the Rekor entry is not a hashedrekord".to_string());
    }
    if text(&["spec", "data", "hash", "algorithm"]) != Some("sha256")
        || text(&["spec", "data", "hash", "value"])
            != Some(hex(&Sha256::digest(checksums)).as_str())
    {
        return Err("the Rekor entry is for a different file".to_string());
    }
    let logged_sig = text(&["spec", "signature", "content"])
        .ok_or_else(|| "the Rekor entry has no signature".to_string())
        .and_then(|s| b64("the Rekor entry signature", s))?;
    if logged_sig != signature {
        return Err("the Rekor entry holds a different signature".to_string());
    }
    let logged_cert = text(&["spec", "signature", "publicKey", "content"])
        .ok_or_else(|| "the Rekor entry has no certificate".to_string())
        .and_then(|s| b64("the Rekor entry certificate", s))
        .and_then(|pem| pem_to_der(&pem))?;
    if logged_cert != leaf_der {
        return Err("the Rekor entry holds a different certificate".to_string());
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Verify that `bundle` is a valid keyless signature of `checksums` for the
/// release `tag` (`vX.Y.Z`, already validated by the caller).
pub(crate) fn verify_bundle(
    checksums: &[u8],
    bundle: &[u8],
    tag: &str,
    trust: &TrustRoots<'_>,
) -> Result<(), String> {
    let bundle: Bundle = serde_json::from_slice(bundle)
        .map_err(|e| format!("the signature bundle is not a valid cosign bundle ({e})"))?;
    let signature = b64("the signature", &bundle.signature)?;
    let leaf_der = pem_to_der(&b64("the bundle certificate", &bundle.cert)?)?;
    let leaf = parse_cert("the signing certificate", &leaf_der)?;

    // 1. The signature over the file, by the leaf's key.
    UnparsedPublicKey::new(
        &ECDSA_P256_SHA256_ASN1,
        leaf.public_key().subject_public_key.data.as_ref(),
    )
    .verify(checksums, &signature)
    .map_err(|_| {
        "the signature does not match checksums-sha256.txt (data mismatch: the file was \
             modified or is not the signed one)"
            .to_string()
    })?;

    // 2-3. Leaf kind and identity.
    check_leaf_usage(&leaf)?;
    check_identity(&leaf, tag)?;

    // 4. Logged time first (it is what the certificate validity is judged at),
    // then the chain at that time.
    let logged_at = check_rekor_timestamp(&bundle.rekor, trust)?;
    let at = ASN1Time::from_timestamp(logged_at)
        .map_err(|_| "the Rekor entry has an invalid time".to_string())?;
    check_chain(&leaf, trust, at)?;

    // 5. The log entry describes this file, signature and certificate.
    check_rekor_body(&bundle.rekor.payload, checksums, &signature, &leaf_der)
}

#[cfg(test)]
mod tests {
    //! The real v0.22.0 release is checked end to end in `upgrade.rs`. The
    //! negative cases need certificates and log entries the real PKI would
    //! never produce (wrong identity, expired leaf, forged entry), so they run
    //! against a throwaway PKI built here with the same shape as Fulcio's.

    use super::*;
    use rcgen::string::Ia5String;
    use rcgen::{
        BasicConstraints, CertificateParams, CustomExtension, DistinguishedName, DnType,
        ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose, SanType, date_time_ymd,
    };
    use ring::rand::SystemRandom;
    use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair as _};
    use serde_json::{Value, json};

    const TAG: &str = "v9.9.9";
    const CHECKSUMS: &[u8] = b"aaaa  uteke-x86_64-apple-darwin-v9.9.9.tar.gz\n";
    /// DER prefix of an ECDSA P-256 SubjectPublicKeyInfo; the 65-byte point follows.
    const P256_SPKI_PREFIX: &[u8] = &[
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
        0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
    ];

    #[derive(Clone)]
    struct Spec {
        san: Vec<String>,
        issuer_v1: Option<&'static str>,
        issuer_v2: Option<&'static str>,
        code_signing: bool,
        digital_signature: bool,
        leaf_is_ca: bool,
        /// Leaf validity, as (from, to) calendar dates; the entry is logged on `logged`.
        valid: ((i32, u8, u8), (i32, u8, u8)),
        logged: (i32, u8, u8),
        /// What the Rekor entry claims, when it should differ from the truth.
        entry_hash: Option<String>,
        entry_signature: Option<Vec<u8>>,
        entry_cert: Option<Vec<u8>>,
    }

    impl Default for Spec {
        fn default() -> Self {
            Spec {
                san: vec![format!("{WORKFLOW_IDENTITY_PREFIX}{TAG}")],
                issuer_v1: Some(GITHUB_ACTIONS_ISSUER),
                issuer_v2: Some(GITHUB_ACTIONS_ISSUER),
                code_signing: true,
                digital_signature: true,
                leaf_is_ca: false,
                valid: ((2026, 10, 30), (2026, 10, 31)),
                logged: (2026, 10, 30),
                entry_hash: None,
                entry_signature: None,
                entry_cert: None,
            }
        }
    }

    struct Pki {
        root_der: Vec<u8>,
        inter_der: Vec<u8>,
        rekor_spki: Vec<u8>,
        bundle: Value,
        rekor_key: EcdsaKeyPair,
    }

    impl Pki {
        fn trust<'a>(
            &'a self,
            roots: &'a [&'a [u8]],
            inter: &'a [&'a [u8]],
            rekor: &'a [&'a [u8]],
        ) -> TrustRoots<'a> {
            TrustRoots {
                roots,
                intermediates: inter,
                rekor_keys: rekor,
            }
        }

        fn check(&self, checksums: &[u8], bundle: &Value, tag: &str) -> Result<(), String> {
            let (r, i, k) = (
                [self.root_der.as_slice()],
                [self.inter_der.as_slice()],
                [self.rekor_spki.as_slice()],
            );
            verify_bundle(
                checksums,
                &serde_json::to_vec(bundle).unwrap(),
                tag,
                &self.trust(&r, &i, &k),
            )
        }

        fn ok(&self) -> Result<(), String> {
            self.check(CHECKSUMS, &self.bundle, TAG)
        }

        /// Re-sign the Rekor timestamp after the payload was edited.
        fn resign(&self, bundle: &mut Value) {
            let p = &bundle["rekorBundle"]["Payload"];
            let canonical = format!(
                "{{\"body\":{},\"integratedTime\":{},\"logID\":{},\"logIndex\":{}}}",
                p["body"], p["integratedTime"], p["logID"], p["logIndex"]
            );
            let sig = self
                .rekor_key
                .sign(&SystemRandom::new(), canonical.as_bytes())
                .unwrap();
            bundle["rekorBundle"]["SignedEntryTimestamp"] = json!(B64.encode(sig.as_ref()));
        }
    }

    fn ca_params(cn: &str, constrained: bool) -> CertificateParams {
        let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
        p.distinguished_name = DistinguishedName::new();
        p.distinguished_name.push(DnType::CommonName, cn);
        p.is_ca = if constrained {
            IsCa::Ca(BasicConstraints::Constrained(0))
        } else {
            IsCa::Ca(BasicConstraints::Unconstrained)
        };
        p.key_usages = vec![KeyUsagePurpose::KeyCertSign];
        p.not_before = date_time_ymd(2020, 1, 1);
        p.not_after = date_time_ymd(2040, 1, 1);
        p
    }

    fn build(spec: &Spec) -> Pki {
        let root_key = KeyPair::generate().unwrap();
        let root_params = ca_params("test-root", false);
        let root = root_params.self_signed(&root_key).unwrap();
        let root_issuer = Issuer::from_params(&root_params, &root_key);

        let inter_key = KeyPair::generate().unwrap();
        let inter_params = ca_params("test-intermediate", true);
        let inter = inter_params.signed_by(&inter_key, &root_issuer).unwrap();
        let inter_issuer = Issuer::from_params(&inter_params, &inter_key);

        let leaf_key = KeyPair::generate().unwrap();
        let mut leaf_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        leaf_params.distinguished_name = DistinguishedName::new();
        leaf_params.subject_alt_names = spec
            .san
            .iter()
            .map(|u| SanType::URI(Ia5String::try_from(u.clone()).unwrap()))
            .collect();
        if spec.digital_signature {
            leaf_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        }
        if spec.code_signing {
            leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::CodeSigning];
        }
        if spec.leaf_is_ca {
            leaf_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        }
        if let Some(v1) = spec.issuer_v1 {
            leaf_params
                .custom_extensions
                .push(CustomExtension::from_oid_content(
                    OID_ISSUER_V1,
                    v1.as_bytes().to_vec(),
                ));
        }
        if let Some(v2) = spec.issuer_v2 {
            let mut der = vec![0x0c, u8::try_from(v2.len()).unwrap()];
            der.extend_from_slice(v2.as_bytes());
            leaf_params
                .custom_extensions
                .push(CustomExtension::from_oid_content(OID_ISSUER_V2, der));
        }
        let ((fy, fm, fd), (ty, tm, td)) = spec.valid;
        leaf_params.not_before = date_time_ymd(fy, fm, fd);
        leaf_params.not_after = date_time_ymd(ty, tm, td);
        let leaf = leaf_params.signed_by(&leaf_key, &inter_issuer).unwrap();
        let leaf_pem = leaf.pem();

        // The file is signed by the leaf key (ECDSA P-256, DER signature).
        let rng = SystemRandom::new();
        let signer = EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_ASN1_SIGNING,
            &leaf_key.serialize_der(),
            &rng,
        )
        .unwrap();
        let signature = signer.sign(&rng, CHECKSUMS).unwrap().as_ref().to_vec();

        let rekor_pkcs8 =
            EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        let rekor_key =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, rekor_pkcs8.as_ref(), &rng)
                .unwrap();
        let mut rekor_spki = P256_SPKI_PREFIX.to_vec();
        rekor_spki.extend_from_slice(rekor_key.public_key().as_ref());

        let entry_cert_pem = spec.entry_cert.as_ref().map_or_else(
            || leaf_pem.clone(),
            |der| {
                let b = B64.encode(der);
                format!("-----BEGIN CERTIFICATE-----\n{b}\n-----END CERTIFICATE-----\n")
            },
        );
        let body = json!({
            "apiVersion": "0.0.1",
            "kind": "hashedrekord",
            "spec": {
                "data": {"hash": {
                    "algorithm": "sha256",
                    "value": spec.entry_hash.clone().unwrap_or_else(|| hex(&Sha256::digest(CHECKSUMS))),
                }},
                "signature": {
                    "content": B64.encode(spec.entry_signature.as_ref().unwrap_or(&signature)),
                    "publicKey": {"content": B64.encode(entry_cert_pem.as_bytes())},
                },
            },
        });
        let logged =
            date_time_ymd(spec.logged.0, spec.logged.1, spec.logged.2).unix_timestamp() + 3600;
        let mut bundle = json!({
            "base64Signature": B64.encode(&signature),
            "cert": B64.encode(leaf_pem.as_bytes()),
            "rekorBundle": {
                "SignedEntryTimestamp": "",
                "Payload": {
                    "body": B64.encode(body.to_string()),
                    "integratedTime": logged,
                    "logIndex": 7,
                    "logID": hex(&Sha256::digest(&rekor_spki)),
                },
            },
        });
        let pki = Pki {
            root_der: root.der().to_vec(),
            inter_der: inter.der().to_vec(),
            rekor_spki,
            bundle: Value::Null,
            rekor_key,
        };
        pki.resign(&mut bundle);
        Pki { bundle, ..pki }
    }

    fn built() -> Pki {
        build(&Spec::default())
    }

    fn err(r: Result<(), String>) -> String {
        r.expect_err("expected the verification to fail")
    }

    #[test]
    fn a_well_formed_bundle_from_the_test_pki_verifies() {
        assert_eq!(built().ok(), Ok(()));
    }

    #[test]
    fn a_changed_file_is_a_data_mismatch() {
        let pki = built();
        let e = err(pki.check(
            b"bbbb  uteke-x86_64-apple-darwin-v9.9.9.tar.gz\n",
            &pki.bundle,
            TAG,
        ));
        assert!(e.contains("data mismatch"), "{e}");
    }

    #[test]
    fn a_signature_by_another_key_is_a_data_mismatch() {
        let pki = built();
        let other = build(&Spec::default());
        let mut bundle = pki.bundle.clone();
        bundle["base64Signature"] = other.bundle["base64Signature"].clone();
        assert!(err(pki.check(CHECKSUMS, &bundle, TAG)).contains("data mismatch"));
    }

    #[test]
    fn the_identity_must_be_this_workflow_at_exactly_this_tag() {
        let pki = built();
        // Same bundle offered for another tag: the replay defence.
        let e = err(pki.check(CHECKSUMS, &pki.bundle, "v9.9.8"));
        assert!(e.contains("different workflow or release"), "{e}");
        for san in [
            "https://github.com/codecoradev/uteke/.github/workflows/other.yml@refs/tags/v9.9.9",
            "https://github.com/someone/uteke/.github/workflows/release.yml@refs/tags/v9.9.9",
            "https://github.com/codecoradev/uteke/.github/workflows/release.yml@refs/heads/main",
            "https://github.com/codecoradev/uteke/.github/workflows/release.yml@refs/tags/v9.9.9x",
        ] {
            let p = build(&Spec {
                san: vec![san.into()],
                ..Spec::default()
            });
            let e = err(p.ok());
            assert!(e.contains("different workflow or release"), "{san}: {e}");
        }
    }

    #[test]
    fn exactly_one_identity_is_accepted() {
        let good = format!("{WORKFLOW_IDENTITY_PREFIX}{TAG}");
        for san in [vec![], vec![good.clone(), "https://example.com/x".into()]] {
            let p = build(&Spec {
                san,
                ..Spec::default()
            });
            assert!(err(p.ok()).contains("identity"), "{}", err(p.ok()));
        }
    }

    #[test]
    fn the_issuer_must_be_github_actions() {
        let evil = "https://accounts.example.com";
        for (v1, v2) in [
            (Some(evil), Some(GITHUB_ACTIONS_ISSUER)),
            (Some(GITHUB_ACTIONS_ISSUER), Some(evil)),
            (None, None),
            (Some(evil), None),
        ] {
            let p = build(&Spec {
                issuer_v1: v1,
                issuer_v2: v2,
                ..Spec::default()
            });
            let e = err(p.ok());
            assert!(
                e.contains("not issued to GitHub Actions"),
                "{v1:?} {v2:?}: {e}"
            );
        }
        // Either form alone is enough, as Fulcio has emitted both.
        for (v1, v2) in [
            (Some(GITHUB_ACTIONS_ISSUER), None),
            (None, Some(GITHUB_ACTIONS_ISSUER)),
        ] {
            let p = build(&Spec {
                issuer_v1: v1,
                issuer_v2: v2,
                ..Spec::default()
            });
            assert_eq!(p.ok(), Ok(()), "{v1:?} {v2:?}");
        }
    }

    #[test]
    fn the_leaf_must_be_a_code_signing_end_entity_certificate() {
        let p = build(&Spec {
            code_signing: false,
            ..Spec::default()
        });
        assert!(err(p.ok()).contains("not a code-signing certificate"));
        let p = build(&Spec {
            digital_signature: false,
            ..Spec::default()
        });
        assert!(err(p.ok()).contains("not allowed to make signatures"));
        let p = build(&Spec {
            leaf_is_ca: true,
            ..Spec::default()
        });
        assert!(err(p.ok()).contains("is a CA"));
    }

    #[test]
    fn the_log_time_must_lie_inside_the_certificate_validity() {
        // Logged after the leaf expired: a leaked ephemeral key signing later.
        let p = build(&Spec {
            logged: (2026, 11, 15),
            ..Spec::default()
        });
        let e = err(p.ok());
        assert!(e.contains("not valid when the signature was logged"), "{e}");
        let p = build(&Spec {
            logged: (2026, 10, 1),
            ..Spec::default()
        });
        assert!(err(p.ok()).contains("not valid when the signature was logged"));
    }

    #[test]
    fn the_leaf_must_chain_to_a_trusted_root() {
        let pki = built();
        let stranger = built();
        let (r, i, k) = (
            [stranger.root_der.as_slice()],
            [stranger.inter_der.as_slice()],
            [pki.rekor_spki.as_slice()],
        );
        let bytes = serde_json::to_vec(&pki.bundle).unwrap();
        let e = err(verify_bundle(
            CHECKSUMS,
            &bytes,
            TAG,
            &pki.trust(&r, &i, &k),
        ));
        assert!(
            e.contains("does not chain to a trusted Sigstore root"),
            "{e}"
        );
        // A real intermediate with the wrong root, and no intermediate at all.
        let (r, i) = ([stranger.root_der.as_slice()], [pki.inter_der.as_slice()]);
        let e = err(verify_bundle(
            CHECKSUMS,
            &bytes,
            TAG,
            &pki.trust(&r, &i, &k),
        ));
        assert!(e.contains("does not chain"), "{e}");
        let none: [&[u8]; 0] = [];
        let e = err(verify_bundle(
            CHECKSUMS,
            &bytes,
            TAG,
            &pki.trust(&r, &none, &k),
        ));
        assert!(e.contains("does not chain"), "{e}");
    }

    #[test]
    fn the_rekor_timestamp_must_verify_against_the_trusted_log_key() {
        let pki = built();
        // Altered timestamp, altered payload fields, altered body.
        let mut b = pki.bundle.clone();
        let sig = b["rekorBundle"]["SignedEntryTimestamp"]
            .as_str()
            .unwrap()
            .to_string();
        let mut raw = B64.decode(sig).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 1;
        b["rekorBundle"]["SignedEntryTimestamp"] = json!(B64.encode(raw));
        assert!(err(pki.check(CHECKSUMS, &b, TAG)).contains("Rekor timestamp does not verify"));
        for (field, value) in [("integratedTime", json!(1)), ("logIndex", json!(8))] {
            let mut b = pki.bundle.clone();
            b["rekorBundle"]["Payload"][field] = value;
            assert!(
                err(pki.check(CHECKSUMS, &b, TAG)).contains("Rekor timestamp does not verify"),
                "{field}"
            );
        }
        let mut b = pki.bundle.clone();
        b["rekorBundle"]["Payload"]["body"] = json!(B64.encode("{}"));
        assert!(err(pki.check(CHECKSUMS, &b, TAG)).contains("Rekor timestamp does not verify"));
        // An entry from a log we do not trust, even when correctly signed by its own key.
        let other = built();
        let e = err(other.check(CHECKSUMS, &pki.bundle, TAG));
        assert!(e.contains("not trusted"), "{e}");
    }

    #[test]
    fn the_rekor_entry_must_describe_this_file_signature_and_certificate() {
        let wrong_hash = build(&Spec {
            entry_hash: Some("0".repeat(64)),
            ..Spec::default()
        });
        assert!(err(wrong_hash.ok()).contains("different file"));
        let wrong_sig = build(&Spec {
            entry_signature: Some(vec![1, 2, 3]),
            ..Spec::default()
        });
        assert!(err(wrong_sig.ok()).contains("different signature"));
        let other_cert = build(&Spec::default());
        let der = pem_to_der(
            &B64.decode(other_cert.bundle["cert"].as_str().unwrap())
                .unwrap(),
        )
        .unwrap();
        let wrong_cert = build(&Spec {
            entry_cert: Some(der),
            ..Spec::default()
        });
        assert!(err(wrong_cert.ok()).contains("different certificate"));
    }

    #[test]
    fn malformed_bundles_are_refused_not_panicked_on() {
        let pki = built();
        let cases: Vec<(&str, Value)> = vec![
            ("missing signature", {
                let mut b = pki.bundle.clone();
                b.as_object_mut().unwrap().remove("base64Signature");
                b
            }),
            ("bad base64", {
                let mut b = pki.bundle.clone();
                b["base64Signature"] = json!("!!!");
                b
            }),
            ("not a certificate", {
                let mut b = pki.bundle.clone();
                b["cert"] = json!(
                    B64.encode("-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n")
                );
                b
            }),
            ("not PEM", {
                let mut b = pki.bundle.clone();
                b["cert"] = json!(B64.encode("hello"));
                b
            }),
            ("wrong PEM label", {
                let mut b = pki.bundle.clone();
                b["cert"] = json!(
                    B64.encode("-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n")
                );
                b
            }),
        ];
        for (name, bundle) in cases {
            assert!(pki.check(CHECKSUMS, &bundle, TAG).is_err(), "{name}");
        }
    }
}
