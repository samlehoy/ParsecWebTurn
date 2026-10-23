//! Rust adaptation of lowlat scripts/kessel-login.py at e548d48.
//! Only account authentication: no daemon, installer, host or extra runtime.
//! MIT attribution: src-native/crates/bud/vendor/lowlat-client/LICENSE.
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{io::Read, time::Duration};

pub struct Login {
    pub email: String,
    pub password: zeroize::Zeroizing<String>,
    pub tfa: zeroize::Zeroizing<String>,
    pub peer: String,
}

// Deliberately no Debug/Serialize: neither token nor password belongs in logs.
pub struct Session(zeroize::Zeroizing<String>);
impl Session {
    pub fn token(&self) -> &str {
        &self.0
    }
    pub fn revoke(&self) {
        // Same DELETE contract as lowlat's --logout. Never log a server body.
        let result = reqwest::blocking::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(1))
            .timeout(Duration::from_secs(3))
            .build()
            .and_then(|client| {
                client
                    .delete("https://kessel-api.parsec.app/auth/sessions")
                    .bearer_auth(self.token())
                    .send()
            });
        if !result.is_ok_and(|response| response.status().is_success()) {
            crate::event_log::record("WARN", "native_account_revoke_failed", json!({}));
        }
    }
}

fn body(login: &Login) -> Value {
    // Same service scopes and optional second-factor contract as lowlat.
    let mut body = json!({"email":login.email.trim(), "password":login.password.as_str(),
        "session_scopes":["ws.host", "ws.client", "api.writer"]});
    if !login.tfa.trim().is_empty() {
        body["tfa"] = json!(login.tfa.trim());
    }
    body
}

fn accepted(status: u16, value: Value) -> Result<Session> {
    if status == 403
        && value["codes"]
            .as_array()
            .is_some_and(|codes| codes.iter().any(|code| code["type"] == "ip_unverified"))
    {
        anyhow::bail!("Confirm the sign-in email from Parsec, then try again.");
    }
    ensure!(
        matches!(status, 200 | 201),
        "Sign-in refused. Check your credentials and two-factor code."
    );
    let token = value["session_id"]
        .as_str()
        .context("No session returned by Parsec")?;
    ensure!(
        !token.is_empty() && token.len() <= 4096 && !token.chars().any(char::is_control),
        "Invalid account session"
    );
    Ok(Session(zeroize::Zeroizing::new(token.into())))
}

pub fn login(login: &Login) -> Result<Session> {
    ensure!(
        !login.email.trim().is_empty() && !login.password.is_empty(),
        "Email and password are required."
    );
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()?;
    let response = client
        .post("https://kessel-api.parsec.app/v2/auth")
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .body(body(login).to_string())
        .send()
        .map_err(|_| anyhow::anyhow!("Could not reach Parsec sign-in securely."))?;
    let status = response.status().as_u16();
    let mut bytes = Vec::new();
    response
        .take(131073)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("Could not read the sign-in response."))?;
    ensure!(bytes.len() <= 131072, "Sign-in response exceeds limit");
    let value =
        serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("Invalid sign-in response."))?;
    accepted(status, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_created_sessions_and_rejects_missing_tokens_without_echoing_secrets() {
        for status in [200, 201] {
            assert_eq!(
                accepted(status, json!({"session_id":"fixture"}))
                    .unwrap()
                    .token(),
                "fixture"
            );
        }
        for status in [302, 401, 403, 500] {
            let error = accepted(
                status,
                json!({"error":"secret-sentinel", "session_id":"secret-sentinel"}),
            )
            .err()
            .unwrap();
            assert!(!error.to_string().contains("secret-sentinel"));
        }
        assert!(accepted(201, json!({})).is_err());
    }
    #[test]
    fn lowlat_auth_shape_and_email_confirmation_are_preserved() {
        let mut login = Login {
            email: " user@example.invalid ".into(),
            password: zeroize::Zeroizing::new("fixture".into()),
            tfa: zeroize::Zeroizing::new(String::new()),
            peer: "1".into(),
        };
        assert!(body(&login).get("tfa").is_none());
        assert_eq!(body(&login)["email"], "user@example.invalid");
        login.tfa = zeroize::Zeroizing::new("012345".into());
        assert_eq!(body(&login)["tfa"], "012345");
        let e = accepted(403, json!({"codes":[{"type":"ip_unverified"}]}))
            .err()
            .unwrap();
        assert!(e.to_string().contains("Confirm"));
    }
}
