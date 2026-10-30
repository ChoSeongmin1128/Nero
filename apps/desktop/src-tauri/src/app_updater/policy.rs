use base64::{engine::general_purpose::STANDARD, Engine};
use minisign_verify::{PublicKey, Signature};
use serde::{Deserialize, Serialize};

/// Product versions stay unchanged; updater wire versions also bind the build.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReleaseIdentity {
    pub version: String,
    pub build: u64,
}
impl ReleaseIdentity {
    pub fn new(version: &str, build: &str) -> Result<Self, String> {
        let parsed =
            semver::Version::parse(version).map_err(|_| "제품 버전을 확인하지 못했습니다.")?;
        if !parsed.pre.is_empty() || !parsed.build.is_empty() || parsed.to_string() != version {
            return Err("제품 버전 형식이 올바르지 않습니다.".into());
        }
        let number = build
            .parse::<u64>()
            .map_err(|_| "빌드 번호가 올바르지 않습니다.")?;
        if number == 0 || number.to_string() != build {
            return Err("빌드 번호가 올바르지 않습니다.".into());
        }
        Ok(Self {
            version: version.into(),
            build: number,
        })
    }
    pub fn parse_wire(value: &str) -> Result<Self, String> {
        let (version, build) = value
            .split_once('+')
            .ok_or("업데이트 빌드 번호가 없습니다.")?;
        Self::new(version, build)
    }
    pub fn wire(&self) -> String {
        format!("{}+{}", self.version, self.build)
    }
    pub fn newer_than(&self, installed: &Self) -> bool {
        // Build numbers increase independently of product versions. SemVer
        // ignores build metadata, so compare the integer explicitly.
        self.build > installed.build
            && semver::Version::parse(&self.version)
                .ok()
                .zip(semver::Version::parse(&installed.version).ok())
                .is_some_and(|(next, current)| next >= current)
    }
}

pub fn verify_package(
    bytes: &[u8],
    public_key: &str,
    signature: &str,
    identity: &ReleaseIdentity,
) -> Result<(), String> {
    let decode = |value: &str| -> Result<String, String> {
        String::from_utf8(
            STANDARD
                .decode(value.trim())
                .map_err(|_| "업데이트 서명 형식이 올바르지 않습니다.")?,
        )
        .map_err(|_| "업데이트 서명 형식이 올바르지 않습니다.".into())
    };
    let key = PublicKey::decode(&decode(public_key)?)
        .map_err(|_| "업데이트 공개키가 올바르지 않습니다.")?;
    let signature = Signature::decode(&decode(signature)?)
        .map_err(|_| "업데이트 서명 형식이 올바르지 않습니다.")?;
    key.verify(bytes, &signature, true)
        .map_err(|_| "업데이트 파일의 서명을 확인하지 못했습니다.")?;
    // Trusted comment is authenticated only after global-signature verification.
    let versions: Vec<_> = signature
        .trusted_comment()
        .split('\t')
        .filter_map(|field| field.strip_prefix("version:"))
        .collect();
    if versions.as_slice() != [identity.wire().as_str()] {
        return Err("서명된 업데이트의 빌드 번호가 일치하지 않습니다.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builds_use_exact_integer_order_without_semver_metadata_ordering() {
        let old = ReleaseIdentity::new("0.0.1", "40").unwrap();
        for build in ["41", "100", "9007199254740993", "18446744073709551615"] {
            let new = ReleaseIdentity::new("0.0.1", build).unwrap();
            assert!(new.newer_than(&old));
            assert_eq!(ReleaseIdentity::parse_wire(&new.wire()).unwrap(), new);
        }
        for build in ["39", "40"] {
            assert!(!ReleaseIdentity::new("0.0.1", build)
                .unwrap()
                .newer_than(&old));
        }
        assert!(ReleaseIdentity::new("0.0.2", "41")
            .unwrap()
            .newer_than(&old));
        assert!(!ReleaseIdentity::new("0.0.2", "39")
            .unwrap()
            .newer_than(&old));
        assert!(!ReleaseIdentity::new("0.0.0", "41")
            .unwrap()
            .newer_than(&old));
    }
    #[test]
    fn ambiguous_overflow_and_unbound_versions_are_rejected() {
        for value in [
            "0.0.1",
            "0.0.1+",
            "0.0.1+0",
            "0.0.1+041",
            "0.0.1+-1",
            "0.0.1+1.0",
            "0.0.1+1e3",
            "0.0.1+18446744073709551616",
            "v0.0.1+41",
            "0.0.1-beta+41",
            "0.0.1+41+42",
        ] {
            assert!(ReleaseIdentity::parse_wire(value).is_err(), "{value}");
        }
    }
    #[test]
    fn real_signatures_bind_bytes_and_the_exact_build() {
        let key = include_str!("fixtures/public-key.txt");
        let payload = include_bytes!("fixtures/payload.txt");
        let signed41 = include_str!("fixtures/bound41.sig");
        let signed40 = include_str!("fixtures/bound40.sig");
        let unbound = include_str!("fixtures/unbound.sig");
        let release41 = ReleaseIdentity::new("0.0.1", "41").unwrap();
        assert!(verify_package(payload, key, signed41, &release41).is_ok());
        assert!(verify_package(b"tampered payload", key, signed41, &release41).is_err());
        assert!(verify_package(payload, key, signed40, &release41).is_err());
        assert!(verify_package(payload, key, unbound, &release41).is_err());
        let comment = String::from_utf8(STANDARD.decode(signed40.trim()).unwrap()).unwrap();
        let relabelled = STANDARD.encode(comment.replace("version:0.0.1+40", "version:0.0.1+41"));
        assert!(verify_package(payload, key, &relabelled, &release41).is_err());
    }
}
