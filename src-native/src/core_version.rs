//! Read-only inspection of the currently served public web core. Never executed
//! or installed. Version labels are an upstream convention, not a stable API.
use std::{
    collections::BTreeSet,
    io::Read,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const SOURCE: &str = "https://web.parsec.app/parsecd";
const MAX_BYTES: usize = 16 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum View {
    Idle,
    Checking,
    Found(String),
    Unavailable(&'static str),
}
struct Cache {
    view: View,
    checked: Option<Instant>,
}
pub struct State(Mutex<Cache>);
impl Default for State {
    fn default() -> Self {
        Self(Mutex::new(Cache {
            view: View::Idle,
            checked: None,
        }))
    }
}
impl State {
    pub fn view(&self) -> View {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .view
            .clone()
    }
    pub fn check(self: &Arc<Self>) {
        if std::env::var_os("PARSECWEBTURN_NO_UPDATE_CHECK").is_some() {
            self.0.lock().unwrap_or_else(|e| e.into_inner()).view =
                View::Unavailable("Online checks are disabled by PARSECWEBTURN_NO_UPDATE_CHECK.");
            return;
        }
        self.check_with(latest);
    }
    fn check_with(
        self: &Arc<Self>,
        fetch: impl FnOnce() -> Result<String, &'static str> + Send + 'static,
    ) {
        {
            let mut cache = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if cache.view == View::Checking
                || (matches!(cache.view, View::Found(_))
                    && cache
                        .checked
                        .is_some_and(|t| t.elapsed() < Duration::from_secs(3600)))
            {
                return;
            }
            cache.view = View::Checking;
        }
        let state = self.clone();
        if std::thread::Builder::new()
            .name("parsec-core-version".into())
            .spawn(move || {
                let result = fetch();
                let mut cache = state.0.lock().unwrap_or_else(|e| e.into_inner());
                cache.view = match result {
                    Ok(version) => View::Found(version),
                    Err(reason) => View::Unavailable(reason),
                };
                cache.checked = Some(Instant::now());
            })
            .is_err()
        {
            self.0.lock().unwrap_or_else(|e| e.into_inner()).view =
                View::Unavailable("Cannot start the version check.");
        }
    }
}

pub fn status(view: &View, embedded: &str) -> String {
    match view {
        View::Idle => "Current public Parsec WASM client: Not checked".into(),
        View::Checking => "Current public Parsec WASM client: Checking...".into(),
        View::Found(version) => format!(
            "Current public Parsec WASM client: {version}\r\n{}",
            if version == embedded {
                "Same version as the embedded core."
            } else {
                "Version differs from the embedded core. No core update is performed."
            }
        ),
        View::Unavailable(reason) => {
            format!("Current public Parsec WASM client: Unavailable\r\n{reason}")
        }
    }
}

fn labels(bytes: &[u8], marker: &[u8], result: &mut BTreeSet<String>) {
    for start in 0..bytes.len().saturating_sub(marker.len()) {
        if !bytes[start..].starts_with(marker) {
            continue;
        }
        let rest = &bytes[start + marker.len()..];
        let Some(end) = rest.iter().position(|b| *b == 0).filter(|n| *n <= 32) else {
            continue;
        };
        let value = &rest[..end];
        let Some(dash) = value.iter().position(|b| *b == b'-') else {
            continue;
        };
        let (major, build) = (&value[..dash], &value[dash + 1..]);
        let digits = build.iter().take_while(|b| b.is_ascii_digit()).count();
        if !major.is_empty()
            && major.iter().all(u8::is_ascii_digit)
            && digits > 0
            && build[digits..].iter().all(u8::is_ascii_lowercase)
        {
            result.insert(String::from_utf8(value.to_vec()).unwrap());
        }
    }
}

fn inspect(mut reader: impl Read) -> Result<String, &'static str> {
    let mut header = [0; 8];
    reader
        .read_exact(&mut header)
        .map_err(|_| "The public core response is incomplete.")?;
    if header != *b"\0asm\x01\0\0\0" {
        return Err("The public core format is not recognized.");
    }
    let mut versions = BTreeSet::new();
    let mut parsec = BTreeSet::new();
    let mut tail = Vec::new();
    let mut chunk = [0; 8192];
    let mut total = header.len();
    loop {
        let count = reader
            .read(&mut chunk)
            .map_err(|_| "The public core version check was interrupted.")?;
        if count == 0 {
            break;
        }
        total += count;
        if total > MAX_BYTES {
            return Err("The public core exceeds the version-check size limit.");
        }
        tail.extend_from_slice(&chunk[..count]);
        labels(&tail, b"Version ", &mut versions);
        labels(&tail, b"Parsec ", &mut parsec);
        if versions.len() > 1 || parsec.len() > 1 {
            return Err("The public core version is ambiguous.");
        }
        let keep = tail.len().saturating_sub(64);
        tail.drain(..keep);
    }
    if versions.len() != 1 || versions != parsec {
        return Err("The public core version labels are not recognized.");
    }
    versions
        .into_iter()
        .next()
        .ok_or("The public core version is unknown.")
}
fn latest() -> Result<String, &'static str> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("ParsecWebTurn/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| "Cannot initialize the public core version check.")?;
    let response = client
        .get(SOURCE)
        .header("Cache-Control", "no-cache")
        .header("Accept-Encoding", "identity")
        .send()
        .map_err(|_| "Cannot reach the public Parsec web client. Reopen About to retry.")?;
    if !response.status().is_success() {
        return Err("The public Parsec web client could not be checked. Reopen About to retry.");
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_BYTES as u64)
    {
        return Err("The public core exceeds the version-check size limit.");
    }
    // Stream and discard bytes, retaining only a tiny boundary window and labels.
    inspect(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn core(labels: &[u8]) -> Vec<u8> {
        [b"\0asm\x01\0\0\0".as_slice(), labels].concat()
    }
    #[test]
    fn labels_must_be_matching_unique_and_bounded_even_across_reads() {
        let mut bytes = core(&vec![0; 8188]);
        bytes.extend_from_slice(b"Version 150-104a\0Parsec 150-104a\0");
        assert_eq!(inspect(bytes.as_slice()).unwrap(), "150-104a");
        for bad in [
            core(b"Version 150-104a\0"),
            core(b"Version 150-104a\0Parsec 150-105a\0"),
            core(b"Version 150-104a\0Parsec 150-104a\0Version 150-105a\0"),
            core(b"Version 150-104a<script>\0Parsec 150-104a<script>\0"),
            b"<html>error</html>".to_vec(),
        ] {
            assert!(inspect(bad.as_slice()).is_err());
        }
        let mut large = core(b"Version 150-104a\0Parsec 150-104a\0");
        large.resize(MAX_BYTES + 1, 0);
        assert!(inspect(large.as_slice()).is_err());
        assert!(status(&View::Found("150-104a".into()), "150-104a").contains("Same version"));
        assert!(status(&View::Found("150-105a".into()), "150-104a").contains("differs"));
        assert!(!status(&View::Unavailable("Offline"), "150-104a").contains("Same version"));
    }
    #[test]
    fn background_check_is_deduplicated_and_success_is_cached() {
        let state = Arc::new(State::default());
        let (tx, rx) = std::sync::mpsc::channel();
        state.check_with(move || {
            rx.recv().unwrap();
            Ok("150-104a".into())
        });
        assert_eq!(state.view(), View::Checking);
        state.check_with(|| panic!("duplicate request"));
        tx.send(()).unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        while state.view() == View::Checking && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(state.view(), View::Found("150-104a".into()));
        state.check_with(|| panic!("cached result should not fetch"));
    }
}
