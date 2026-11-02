use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Deserialize;
use serde_json::Value;

pub(super) const ISSUER: &str = "https://auth.openai.com";
pub(super) const DISCOVERY: &str = "https://auth.openai.com/.well-known/openid-configuration";
pub const RESOURCE: &str = "https://api.openai.com/v1";
pub(super) const DIRECT_SCOPE: &str = "chatgpt.tokens.use.direct";
pub(super) const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const INVALID: &str = "ChatGPT 계정의 인증 정보를 확인하지 못했습니다. 다시 로그인해 주세요.";

#[derive(Deserialize)]
pub(super) struct Discovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
    pub revocation_endpoint: String,
}
impl Discovery {
    pub fn validate(&self) -> Result<(), String> {
        if self.issuer != ISSUER
            || self.authorization_endpoint != "https://auth.openai.com/api/accounts/authorize"
            || self.token_endpoint != "https://auth.openai.com/api/accounts/oauth/token"
        {
            return Err(INVALID.into());
        }
        for value in [&self.jwks_uri, &self.revocation_endpoint] {
            let url = reqwest::Url::parse(value).map_err(|_| INVALID)?;
            if url.scheme() != "https"
                || url.host_str() != Some("auth.openai.com")
                || !url.username().is_empty()
                || url.password().is_some()
                || url.port().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(INVALID.into());
            }
        }
        if self.jwks_uri != "https://auth.openai.com/.well-known/jwks.json" {
            return Err(INVALID.into());
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct Header {
    alg: String,
    kid: String,
    #[serde(default)]
    crit: Vec<String>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}
#[derive(Deserialize)]
struct Claims {
    iss: String,
    sub: String,
    aud: Audience,
    exp: u64,
    iat: u64,
    #[serde(default)]
    nbf: Option<u64>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    at_hash: Option<String>,
}
#[derive(Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}
#[derive(Deserialize)]
struct Jwk {
    kty: String,
    kid: String,
    n: String,
    e: String,
    #[serde(default)]
    alg: Option<String>,
    #[serde(default, rename = "use")]
    usage: Option<String>,
    #[serde(default)]
    key_ops: Option<Vec<String>>,
}

pub(super) struct Identity {
    pub subject: String,
    pub email: Option<String>,
    pub name: Option<String>,
}

pub(super) fn verify_id_token(
    token: &str,
    jwks: &Value,
    client_id: &str,
    nonce: Option<&str>,
    access_token: &str,
    now: u64,
) -> Result<Identity, String> {
    if token.len() > 96 * 1024 {
        return Err(INVALID.into());
    }
    let parts: Vec<_> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(INVALID.into());
    }
    let header: Header = decode(parts[0])?;
    if header.alg != "RS256" || header.kid.is_empty() || !header.crit.is_empty() {
        return Err(INVALID.into());
    }
    let keys: Jwks = serde_json::from_value(jwks.clone()).map_err(|_| INVALID)?;
    let matching: Vec<_> = keys
        .keys
        .iter()
        .filter(|key| key.kid == header.kid)
        .collect();
    if matching.len() != 1 {
        return Err(INVALID.into());
    }
    let key = matching[0];
    if key.kty != "RSA"
        || key.alg.as_deref().is_some_and(|alg| alg != "RS256")
        || key.usage.as_deref().is_some_and(|usage| usage != "sig")
        || key
            .key_ops
            .as_ref()
            .is_some_and(|ops| !ops.iter().any(|op| op == "verify"))
    {
        return Err(INVALID.into());
    }
    let n = URL_SAFE_NO_PAD.decode(&key.n).map_err(|_| INVALID)?;
    let e = URL_SAFE_NO_PAD.decode(&key.e).map_err(|_| INVALID)?;
    let signature = URL_SAFE_NO_PAD.decode(parts[2]).map_err(|_| INVALID)?;
    let signed = format!("{}.{}", parts[0], parts[1]);
    ring::signature::RsaPublicKeyComponents { n: &n, e: &e }
        .verify(
            &ring::signature::RSA_PKCS1_2048_8192_SHA256,
            signed.as_bytes(),
            &signature,
        )
        .map_err(|_| INVALID)?;
    let claims: Claims = decode(parts[1])?;
    validate_claims(claims, client_id, nonce, access_token, now)
}

fn validate_claims(
    claims: Claims,
    client_id: &str,
    nonce: Option<&str>,
    access_token: &str,
    now: u64,
) -> Result<Identity, String> {
    let audiences = match &claims.aud {
        Audience::One(value) => vec![value.as_str()],
        Audience::Many(values) => values.iter().map(String::as_str).collect(),
    };
    if claims.iss != ISSUER
        || audiences.len() != 1
        || audiences[0] != client_id
        || claims.exp <= now
        || claims.iat > now
        || claims.nbf.is_some_and(|nbf| nbf > now)
        || claims.subject_invalid()
    {
        return Err(INVALID.into());
    }
    if nonce.is_some_and(|expected| claims.nonce.as_deref() != Some(expected)) {
        return Err(INVALID.into());
    }
    if let Some(expected) = &claims.at_hash {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(access_token.as_bytes());
        if URL_SAFE_NO_PAD.encode(&digest[..digest.len() / 2]) != *expected {
            return Err(INVALID.into());
        }
    }
    Ok(Identity {
        subject: claims.sub,
        email: clean_label(claims.email),
        name: clean_label(claims.name),
    })
}
impl Claims {
    fn subject_invalid(&self) -> bool {
        self.sub.is_empty() || self.sub.len() > 1024 || self.sub.chars().any(char::is_control)
    }
}
fn clean_label(value: Option<String>) -> Option<String> {
    value
        .map(|value| {
            value
                .chars()
                .filter(|ch| !ch.is_control())
                .take(256)
                .collect::<String>()
        })
        .filter(|value| !value.trim().is_empty())
}
fn decode<T: for<'de> Deserialize<'de>>(value: &str) -> Result<T, String> {
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(value).map_err(|_| INVALID)?)
        .map_err(|_| INVALID.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn claims() -> Claims {
        serde_json::from_value(serde_json::json!({"iss":ISSUER,"sub":"fixture","aud":"oaiapp_fixture","exp":200,"iat":50,"nonce":"nonce","email":"same@example.invalid"})).unwrap()
    }
    #[test]
    fn claims_require_exact_issuer_audience_time_and_nonce() {
        assert!(validate_claims(claims(), "oaiapp_fixture", Some("nonce"), "fixture", 100).is_ok());
        assert!(validate_claims(claims(), "other", Some("nonce"), "fixture", 100).is_err());
        assert!(
            validate_claims(claims(), "oaiapp_fixture", Some("wrong"), "fixture", 100).is_err()
        );
        assert!(
            validate_claims(claims(), "oaiapp_fixture", Some("nonce"), "fixture", 200).is_err()
        );
        let mut wrong = claims();
        wrong.iss = "https://other.invalid".into();
        assert!(validate_claims(wrong, "oaiapp_fixture", Some("nonce"), "fixture", 100).is_err());
        let mut future = claims();
        future.iat = 101;
        assert!(validate_claims(future, "oaiapp_fixture", Some("nonce"), "fixture", 100).is_err());
    }
    #[test]
    fn unverified_algorithms_unknown_keys_and_modified_signatures_fail_closed() {
        let payload=URL_SAFE_NO_PAD.encode(serde_json::json!({"iss":ISSUER,"aud":"client","sub":"fixture","exp":99999,"iat":1,"nonce":"nonce"}).to_string());
        for header in [
            serde_json::json!({"alg":"none","kid":"fixture"}),
            serde_json::json!({"alg":"HS256","kid":"fixture"}),
            serde_json::json!({"alg":"RS256","kid":"missing"}),
        ] {
            let token = format!(
                "{}.{}.AA",
                URL_SAFE_NO_PAD.encode(header.to_string()),
                payload
            );
            assert!(verify_id_token(
                &token,
                &serde_json::json!({"keys":[]}),
                "client",
                Some("nonce"),
                "fixture",
                100
            )
            .is_err());
        }
        let token = format!(
            "{}.{}.AA",
            URL_SAFE_NO_PAD.encode(serde_json::json!({"alg":"RS256","kid":"fixture"}).to_string()),
            payload
        );
        assert!(verify_id_token(&token,&serde_json::json!({"keys":[{"kty":"RSA","kid":"fixture","n":"AQAB","e":"AQAB","alg":"RS256","use":"sig"}]}),"client",Some("nonce"),"fixture",100).is_err());
    }
    #[test]
    fn discovery_cannot_redirect_credentials_to_untrusted_origins() {
        let discovery = Discovery {
            issuer: ISSUER.into(),
            authorization_endpoint: "https://auth.openai.com/api/accounts/authorize".into(),
            token_endpoint: "https://auth.openai.com/api/accounts/oauth/token".into(),
            jwks_uri: "https://auth.openai.com/.well-known/jwks.json".into(),
            revocation_endpoint: "https://other.invalid/revoke".into(),
        };
        assert!(discovery.validate().is_err());
    }
    #[test]
    fn published_public_key_fixture_verifies_signature_before_returning_identity() {
        let fixture: Value =
            serde_json::from_str(include_str!("testdata/oidc-rs256.json")).unwrap();
        let token = fixture["token"].as_str().unwrap();
        let keys = &fixture["jwks"];
        let time = fixture["now"].as_u64().unwrap();
        let identity = verify_id_token(
            token,
            keys,
            "oaiapp_fixture",
            Some("synthetic-nonce"),
            "fixture-access",
            time,
        )
        .unwrap();
        assert_eq!(identity.subject, "synthetic-subject");
        assert!(verify_id_token(
            token,
            keys,
            "oaiapp_other",
            Some("synthetic-nonce"),
            "fixture-access",
            time
        )
        .is_err());
        assert!(verify_id_token(
            token,
            keys,
            "oaiapp_fixture",
            Some("wrong"),
            "fixture-access",
            time
        )
        .is_err());
        let mut parts: Vec<String> = token.split('.').map(String::from).collect();
        parts[1]=URL_SAFE_NO_PAD.encode(serde_json::json!({"iss":ISSUER,"aud":"oaiapp_fixture","sub":"forged","iat":1,"exp":time+1000,"nonce":"synthetic-nonce"}).to_string());
        assert!(verify_id_token(
            &parts.join("."),
            keys,
            "oaiapp_fixture",
            Some("synthetic-nonce"),
            "fixture-access",
            time
        )
        .is_err());
    }
}
