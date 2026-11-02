use super::{now, valid_secret, Tokens};
use serde::Deserialize;

// Compatible public OAuth contract from Pi (MIT), pinned c9f4c7b9fcd0f3e6b5bbc35c7c9b6cda35580eb8.
pub(super) const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
pub(super) const AUTHORIZE: &str = "https://claude.ai/oauth/authorize";
pub(super) const TOKEN: &str = "https://platform.claude.com/v1/oauth/token";
pub(super) const SCOPES: &str = "user:profile user:inference";
pub(super) const BASE_URL: &str = "https://api.anthropic.com";

#[derive(Deserialize)]
pub(super) struct Response {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    account: Option<Account>,
}
#[derive(Deserialize)]
struct Account {
    uuid: String,
    #[serde(default)]
    email_address: Option<String>,
}
impl Response {
    pub fn identity(&self) -> Option<(String, Option<String>)> {
        let account = self.account.as_ref()?;
        if account.uuid.is_empty()
            || account.uuid.len() > 256
            || account.uuid.chars().any(char::is_control)
        {
            return None;
        }
        Some((
            account.uuid.clone(),
            account
                .email_address
                .as_ref()
                .filter(|email| email.len() <= 256 && !email.chars().any(char::is_control))
                .cloned(),
        ))
    }
    pub fn into_tokens(self, previous: Option<&Tokens>) -> Result<Tokens, String> {
        if !self.access_token.starts_with("sk-ant-oat")
            || !valid_secret(&self.access_token)
            || !valid_secret(&self.refresh_token)
            || self.expires_in == 0
        {
            return Err("Claude 구독 인증 응답을 확인하지 못했습니다.".into());
        }
        let scopes = match self.scope {
            Some(scope) => scope.split_whitespace().map(String::from).collect(),
            None => previous
                .map(|tokens| tokens.scopes.clone())
                .unwrap_or_else(|| SCOPES.split_whitespace().map(String::from).collect()),
        };
        let expires_at = now()
            .checked_add(self.expires_in)
            .ok_or("Claude 인증 만료 시간을 확인하지 못했습니다.")?;
        Ok(Tokens {
            access_token: self.access_token,
            refresh_token: Some(self.refresh_token),
            id_token: None,
            scopes,
            expires_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_oauth_subscription_tokens_are_accepted_and_no_api_key_scope_is_requested() {
        assert!(!SCOPES.contains("api_key"));
        assert!(!SCOPES.contains("file_upload"));
        let response:Response=serde_json::from_value(serde_json::json!({"access_token":"sk-ant-api03-fixture","refresh_token":"fixture","expires_in":3600})).unwrap();
        assert!(response.into_tokens(None).is_err());
        let response:Response=serde_json::from_value(serde_json::json!({"access_token":"sk-ant-oat01-fixture","refresh_token":"replacement","expires_in":3600})).unwrap();
        let tokens = response.into_tokens(None).unwrap();
        assert_eq!(tokens.refresh_token.as_deref(), Some("replacement"));
        assert!(tokens.scopes.iter().any(|scope| scope == "user:inference"));
    }
    #[test]
    fn granted_scope_removal_does_not_restore_inference_from_requested_scopes() {
        let response:Response=serde_json::from_value(serde_json::json!({"access_token":"sk-ant-oat01-fixture","refresh_token":"replacement","expires_in":3600,"scope":"user:profile"})).unwrap();
        let tokens = response.into_tokens(None).unwrap();
        assert!(!tokens.scopes.iter().any(|scope| scope == "user:inference"));
    }
}
