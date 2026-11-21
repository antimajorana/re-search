//! ID-token validation: RS256 signature against the issuer's JWKS (`ring`), then issuer,
//! audience (the issued client ID), expiry, `iat`, nonce and subject, as OpenAI's Sign in
//! with ChatGPT documentation requires ("Validate the ID token and ChatGPT plan
//! permission"; the website guide's example uses a 5-second clock tolerance).
//!
//! Errors are fixed descriptions: they never contain the token or any claim value.

use crate::codec::{b64url_decode, ct_eq};
use ring::signature::{RsaPublicKeyComponents, RSA_PKCS1_2048_8192_SHA256};
use serde_json::Value;
use std::fmt;

/// Allowed clock skew in seconds (the documentation's website example uses 5).
pub const CLOCK_SKEW_S: i64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdTokenError {
    Malformed,
    UnsupportedAlgorithm,
    UnknownKey,
    BadSignature,
    WrongIssuer,
    WrongAudience,
    Expired,
    MissingIssuedAt,
    WrongNonce,
    MissingSubject,
}

impl fmt::Display for IdTokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            IdTokenError::Malformed => "ID token is malformed",
            IdTokenError::UnsupportedAlgorithm => "ID token is not signed with RS256",
            IdTokenError::UnknownKey => "ID token is signed with a key that is not in OpenAI's published key set",
            IdTokenError::BadSignature => "ID token signature is invalid",
            IdTokenError::WrongIssuer => "ID token issuer is not OpenAI's",
            IdTokenError::WrongAudience => "ID token was not issued to this app's client ID",
            IdTokenError::Expired => "ID token has expired",
            IdTokenError::MissingIssuedAt => "ID token has no issued-at time",
            IdTokenError::WrongNonce => "ID token nonce does not match this sign-in attempt",
            IdTokenError::MissingSubject => "ID token has no subject",
        })
    }
}

impl std::error::Error for IdTokenError {}

/// Claims kept after validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdClaims {
    pub sub: String,
    /// Only when `email_verified` is the boolean `true` (as the documentation advises).
    pub verified_email: Option<String>,
    pub exp: i64,
}

fn decode_json(part: &str) -> Result<Value, IdTokenError> {
    let bytes = b64url_decode(part).ok_or(IdTokenError::Malformed)?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|_| IdTokenError::Malformed)?;
    if v.is_object() {
        Ok(v)
    } else {
        Err(IdTokenError::Malformed)
    }
}

/// The RSA key (`n`, `e`) for `kid` in a JWKS document (`{"keys": [...]}`); without a
/// `kid` in the token header, the key set must hold exactly one usable RSA key.
fn find_key(jwks: &Value, kid: Option<&str>) -> Result<(Vec<u8>, Vec<u8>), IdTokenError> {
    let keys = jwks.get("keys").and_then(Value::as_array).ok_or(IdTokenError::UnknownKey)?;
    let usable: Vec<&Value> = keys
        .iter()
        .filter(|k| k.get("kty").and_then(Value::as_str) == Some("RSA"))
        .filter(|k| k.get("use").and_then(Value::as_str).is_none_or(|u| u == "sig"))
        .filter(|k| k.get("alg").and_then(Value::as_str).is_none_or(|a| a == "RS256"))
        .collect();
    let key = match kid {
        Some(kid) => usable.into_iter().find(|k| k.get("kid").and_then(Value::as_str) == Some(kid)),
        None if usable.len() == 1 => usable.into_iter().next(),
        None => None,
    }
    .ok_or(IdTokenError::UnknownKey)?;
    let part = |name: &str| {
        key.get(name).and_then(Value::as_str).and_then(b64url_decode).ok_or(IdTokenError::UnknownKey)
    };
    Ok((part("n")?, part("e")?))
}

/// Validate `token` (compact JWS). `now_s` is the current Unix time.
pub fn verify_id_token(
    token: &str,
    jwks: &Value,
    issuer: &str,
    client_id: &str,
    nonce: &str,
    now_s: i64,
) -> Result<IdClaims, IdTokenError> {
    let mut parts = token.split('.');
    let (Some(h), Some(p), Some(s), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(IdTokenError::Malformed);
    };
    let header = decode_json(h)?;
    if header.get("alg").and_then(Value::as_str) != Some("RS256") {
        return Err(IdTokenError::UnsupportedAlgorithm);
    }
    if header.get("crit").is_some() {
        return Err(IdTokenError::Malformed);
    }
    let (n, e) = find_key(jwks, header.get("kid").and_then(Value::as_str))?;
    let sig = b64url_decode(s).ok_or(IdTokenError::Malformed)?;
    let signing_input = &token.as_bytes()[..h.len() + 1 + p.len()];
    RsaPublicKeyComponents { n: &n, e: &e }
        .verify(&RSA_PKCS1_2048_8192_SHA256, signing_input, &sig)
        .map_err(|_| IdTokenError::BadSignature)?;

    let claims = decode_json(p)?;
    if claims.get("iss").and_then(Value::as_str) != Some(issuer) {
        return Err(IdTokenError::WrongIssuer);
    }
    let aud_ok = match claims.get("aud") {
        Some(Value::String(a)) => a == client_id,
        Some(Value::Array(list)) => list.iter().any(|a| a.as_str() == Some(client_id)),
        _ => false,
    };
    // OIDC Core 3.1.3.7: with several audiences, `azp` (when present) must be this client.
    let azp_ok = claims.get("azp").and_then(Value::as_str).is_none_or(|azp| azp == client_id);
    if !aud_ok || !azp_ok {
        return Err(IdTokenError::WrongAudience);
    }
    let exp = claims.get("exp").and_then(Value::as_i64).ok_or(IdTokenError::Expired)?;
    if exp + CLOCK_SKEW_S <= now_s {
        return Err(IdTokenError::Expired);
    }
    if claims.get("iat").and_then(Value::as_i64).is_none() {
        return Err(IdTokenError::MissingIssuedAt);
    }
    let got_nonce = claims.get("nonce").and_then(Value::as_str).unwrap_or("");
    if got_nonce.is_empty() || !ct_eq(got_nonce.as_bytes(), nonce.as_bytes()) {
        return Err(IdTokenError::WrongNonce);
    }
    let sub = claims.get("sub").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).ok_or(IdTokenError::MissingSubject)?;
    let verified_email = match (claims.get("email").and_then(Value::as_str), claims.get("email_verified")) {
        (Some(e), Some(Value::Bool(true))) if !e.is_empty() => Some(e.chars().take(254).collect()),
        _ => None,
    };
    Ok(IdClaims { sub: sub.to_string(), verified_email, exp })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::b64url_encode;
    use crate::testkit::{id_claims, jwks, sign_jwt, sign_jwt_with_header};
    use serde_json::json;

    const ISS: &str = "https://auth.example.test";
    const NOW: i64 = 2_000_000_000;

    fn check(token: &str) -> Result<IdClaims, IdTokenError> {
        verify_id_token(token, &jwks(), ISS, "oaiapp_1", "nonce-1", NOW)
    }

    #[test]
    fn a_valid_token_passes_and_keeps_only_a_verified_email() {
        let c = check(&sign_jwt(&id_claims(ISS, "oaiapp_1", "nonce-1", "sub-1", NOW + 600))).unwrap();
        assert_eq!(c.sub, "sub-1");
        assert_eq!(c.verified_email.as_deref(), Some("person@example.test"));
        let mut claims = id_claims(ISS, "oaiapp_1", "nonce-1", "sub-1", NOW + 600);
        claims["email_verified"] = json!(false);
        claims["aud"] = json!("oaiapp_1");
        assert_eq!(check(&sign_jwt(&claims)).unwrap().verified_email, None);
    }

    #[test]
    fn each_failure_is_reported_by_class() {
        let good = id_claims(ISS, "oaiapp_1", "nonce-1", "sub-1", NOW + 600);
        let with = |k: &str, v: Value| {
            let mut c = good.clone();
            c[k] = v;
            sign_jwt(&c)
        };
        let token = sign_jwt(&good);
        let (head, sig) = token.rsplit_once('.').unwrap();
        let mut bad_sig = crate::codec::b64url_decode(sig).unwrap();
        bad_sig[10] ^= 1;
        assert_eq!(check(&format!("{head}.{}", b64url_encode(&bad_sig))), Err(IdTokenError::BadSignature));
        // A payload swapped under a valid signature.
        let (h, _) = head.split_once('.').unwrap();
        let forged_payload = with("sub", json!("attacker"));
        let p2 = forged_payload.split('.').nth(1).unwrap();
        assert_eq!(check(&format!("{h}.{p2}.{sig}")), Err(IdTokenError::BadSignature));
        assert_eq!(check(&with("aud", json!(["oaiapp_other"]))), Err(IdTokenError::WrongAudience));
        assert_eq!(check(&with("nonce", json!("nonce-2"))), Err(IdTokenError::WrongNonce));
        assert_eq!(check(&with("exp", json!(NOW - 60))), Err(IdTokenError::Expired));
        assert_eq!(check(&with("iss", json!("https://evil.example"))), Err(IdTokenError::WrongIssuer));
        assert_eq!(check(&with("sub", json!(""))), Err(IdTokenError::MissingSubject));
        let unknown = sign_jwt_with_header(&json!({ "alg": "RS256", "kid": "other" }), &good);
        assert_eq!(check(&unknown), Err(IdTokenError::UnknownKey));
        let none_alg = format!("{}.{}.", b64url_encode(br#"{"alg":"none"}"#), b64url_encode(good.to_string().as_bytes()));
        assert_eq!(check(&none_alg), Err(IdTokenError::UnsupportedAlgorithm));
        assert_eq!(check("a.b"), Err(IdTokenError::Malformed));
        // Messages never contain claim values.
        assert!(!IdTokenError::WrongAudience.to_string().contains("oaiapp"));
    }
}
