//! Typed provider error taxonomy with user-action hints, shared across all providers.

use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorClass {
    AuthExpired,
    MissingCredential,
    PermissionDenied,
    RateLimited,
    ProviderUnavailable,
    ParseFailure,
    NetworkFailure,
    ApiFailure,
    Timeout,
    NotInstalled,
}

impl ErrorClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorClass::AuthExpired => "authentication-expired",
            ErrorClass::MissingCredential => "missing-credential",
            ErrorClass::PermissionDenied => "permission-denied",
            ErrorClass::RateLimited => "rate-limited",
            ErrorClass::ProviderUnavailable => "provider-unavailable",
            ErrorClass::ParseFailure => "parse-failure",
            ErrorClass::NetworkFailure => "network-failure",
            ErrorClass::ApiFailure => "api-failure",
            ErrorClass::Timeout => "timeout",
            ErrorClass::NotInstalled => "not-installed",
        }
    }
}

/// A provider poll failure. `message` must never contain credential material
/// (see `secret::SecretString`); it is what gets rendered to users.
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderError {
    pub class: ErrorClass,
    pub message: String,
    /// Background cooldown this provider should observe before the next poll
    /// (e.g. 300 s after an HTTP 429 or a resolver timeout).
    pub cooldown: Option<Duration>,
}

impl ProviderError {
    pub fn new(class: ErrorClass, message: impl Into<String>) -> Self {
        ProviderError {
            class,
            message: message.into(),
            cooldown: None,
        }
    }

    pub fn with_cooldown(mut self, d: Duration) -> Self {
        self.cooldown = Some(d);
        self
    }

    /// One-line action the user can take, keyed by class.
    pub fn user_hint(&self, provider_hint: &str) -> String {
        match self.class {
            ErrorClass::AuthExpired => {
                format!("re-authenticate in {} to refresh this token", provider_hint)
            }
            ErrorClass::MissingCredential => format!("sign in with {} first", provider_hint),
            ErrorClass::RateLimited => "backing off; try a manual refresh later".to_string(),
            _ => self.message.clone(),
        }
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.class.as_str(), self.message)
    }
}

impl std::error::Error for ProviderError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_strings_are_stable() {
        assert_eq!(ErrorClass::AuthExpired.as_str(), "authentication-expired");
        assert_eq!(ErrorClass::RateLimited.as_str(), "rate-limited");
    }

    #[test]
    fn hint_mentions_reauth_for_expired() {
        let e = ProviderError::new(ErrorClass::AuthExpired, "token expired at read time");
        assert!(e.user_hint("the codex CLI").contains("re-authenticate"));
    }
}
