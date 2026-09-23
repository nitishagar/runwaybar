//! A string that never renders its contents through `Debug`, `Display`, or JSON.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, PartialEq, Eq)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(s: impl Into<String>) -> Self {
        SecretString(s.into())
    }

    /// The only way to read the material. Use at the boundary that needs it
    /// (an HTTP header); never let the result flow into errors, logs or JSON.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretString(<redacted, {} bytes>)", self.0.len())
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<redacted>")
    }
}

// Serialized form stays redacted too: snapshots/IPC must never carry secrets.
impl Serialize for SecretString {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str("<redacted>")
    }
}

impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Ok(SecretString::new(raw))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MATERIAL: &str = "sk-super-secret-1234567890";

    #[test]
    fn debug_and_display_never_leak() {
        let s = SecretString::new(MATERIAL);
        let dbg = format!("{s:?}");
        let disp = format!("{s}");
        assert!(!dbg.contains(MATERIAL), "debug leaked: {dbg}");
        assert!(!disp.contains(MATERIAL), "display leaked: {disp}");
        assert!(dbg.contains("redacted"));
    }

    #[test]
    fn json_serialization_stays_redacted() {
        let s = SecretString::new(MATERIAL);
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains(MATERIAL), "json leaked: {json}");
    }
}
