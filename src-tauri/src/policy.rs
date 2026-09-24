use url::Url;

pub fn valid_environment(origin: &str, channel: &str, scheme: &str) -> bool {
    matches!(
        (origin, channel, scheme),
        ("https://access.cognuum.com", "production", "cognuum")
            | (
                "https://staging-access.cognuum.com",
                "staging",
                "cognuum-staging"
            )
    )
}

pub fn same_origin(url: &Url, origin: &str) -> bool {
    url.origin().ascii_serialization() == origin
        && url.username().is_empty()
        && url.password().is_none()
}

pub fn external_url(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
}

/// Only one PKCE authorization code is accepted. Never accept sessions, tokens,
/// arbitrary destinations, or a callback from another installation channel.
pub fn auth_callback(url: &Url, scheme: &str, origin: &str) -> Option<Url> {
    if url.scheme() != scheme
        || url.host_str() != Some("auth")
        || url.path() != "/callback"
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }
    let params: Vec<_> = url.query_pairs().collect();
    if params.len() != 1 || params[0].0 != "code" {
        return None;
    }
    let code = &params[0].1;
    if code.is_empty()
        || code.len() > 2048
        || !code
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return None;
    }
    let mut target = Url::parse(&format!("{origin}/login")).ok()?;
    target.query_pairs_mut().append_pair("desktop_code", code);
    Some(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    const PROD: &str = "https://access.cognuum.com";
    #[test]
    fn environment_is_exact_and_channel_bound() {
        assert!(valid_environment(PROD, "production", "cognuum"));
        assert!(!valid_environment(
            "https://dev-access.cognuum.com",
            "production",
            "cognuum"
        ));
        assert!(!valid_environment(PROD, "staging", "cognuum-staging"));
    }
    #[test]
    fn navigation_rejects_lookalikes_and_credentials() {
        assert!(same_origin(
            &Url::parse(&format!("{PROD}/console")).unwrap(),
            PROD
        ));
        for raw in [
            "http://access.cognuum.com",
            "https://access.cognuum.com.evil.test",
            "https://user@access.cognuum.com",
        ] {
            assert!(!same_origin(&Url::parse(raw).unwrap(), PROD));
        }
        assert!(external_url(
            &Url::parse("https://example.org/report").unwrap()
        ));
        for raw in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "cognuum://auth/callback?code=abc",
            "https://user@example.org",
        ] {
            assert!(!external_url(&Url::parse(raw).unwrap()));
        }
    }
    #[test]
    fn callback_accepts_only_pkce_codes_for_this_channel() {
        assert_eq!(
            auth_callback(
                &Url::parse("cognuum://auth/callback?code=abc-123").unwrap(),
                "cognuum",
                PROD
            )
            .unwrap()
            .as_str(),
            "https://access.cognuum.com/login?desktop_code=abc-123"
        );
        for raw in [
            "cognuum-staging://auth/callback?code=abc",
            "cognuum://evil/callback?code=abc",
            "cognuum://auth/callback?code=abc&code=xyz",
            "cognuum://auth/callback?access_token=abc",
            "cognuum://auth/callback?code=abc#access_token=secret",
            "cognuum://auth/callback?code=abc&redirect=https://evil.test",
            "cognuum://auth/callback?code=",
        ] {
            assert!(
                auth_callback(&Url::parse(raw).unwrap(), "cognuum", PROD).is_none(),
                "{raw}"
            );
        }
    }
}
