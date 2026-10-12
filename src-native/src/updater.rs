//! Metadata-only stable release checks, following the main branch updater.
//! Downloads and installation are performed by the user through their browser.
use serde::Deserialize;
use std::{
    io::Read,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

const API: &str = "https://api.github.com/repos/vampywiz17/ParsecWebTurn/releases/latest";
const REPOSITORY: &str = "https://github.com/vampywiz17/ParsecWebTurn";
const MAX_METADATA: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Checking,
    Current,
    Available,
    Error,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub phase: Phase,
    pub message: &'static str,
    pub release: Option<Available>,
}
impl Default for View {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            message: "Ready to check for a newer stable GitHub release.",
            release: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Available {
    pub version: String,
    pub notes: String,
    pub download_url: String,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    body: Option<String>,
    assets: Vec<Asset>,
}

#[derive(Default)]
pub struct State {
    view: Mutex<View>,
    notification: AtomicBool,
}
impl State {
    pub fn view(&self) -> View {
        self.view.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub fn take_notification(&self) -> bool {
        self.notification.swap(false, Ordering::AcqRel)
    }
    pub fn check(self: &Arc<Self>, notify: bool) {
        self.check_with(notify, latest);
    }
    fn check_with(
        self: &Arc<Self>,
        notify: bool,
        fetch: impl FnOnce() -> Result<Option<Available>, &'static str> + Send + 'static,
    ) {
        {
            let mut view = self.view.lock().unwrap_or_else(|e| e.into_inner());
            if view.phase == Phase::Checking {
                return;
            }
            *view = View {
                phase: Phase::Checking,
                message: "Checking the latest stable GitHub release...",
                release: None,
            };
            self.notification.store(false, Ordering::Release);
        }
        let state = self.clone();
        if std::thread::Builder::new().name("parsec-update-check".into()).spawn(move || {
            let result = fetch();
            let mut view = state.view.lock().unwrap_or_else(|e| e.into_inner());
            *view = match result {
                Ok(Some(release)) => View { phase: Phase::Available,
                    message: "A newer stable version is available. Download the ZIP in your browser, then close the app and replace the EXE manually.", release: Some(release) },
                Ok(None) => View { phase: Phase::Current,
                    message: "No newer stable release is available. Development builds may be ahead of the latest stable release.", release: None },
                Err(message) => View { phase: Phase::Error, message, release: None },
            };
            state.notification.store(notify && view.phase == Phase::Available, Ordering::Release);
        }).is_err() {
            *self.view.lock().unwrap_or_else(|e| e.into_inner()) = View {
                phase: Phase::Error, message: "Cannot start the update check. Try again later.", release: None,
            };
        }
    }
}

fn stable_version(value: &str) -> Result<[u64; 3], &'static str> {
    let mut parts = value.strip_prefix('v').unwrap_or(value).split('.');
    let mut numbers = [0; 3];
    for number in &mut numbers {
        *number = numeric(parts.next().ok_or("Invalid release version.")?)?;
    }
    if parts.next().is_some() {
        return Err("Invalid release version.");
    }
    Ok(numbers)
}
fn numeric(value: &str) -> Result<u64, &'static str> {
    if value.is_empty()
        || !value.bytes().all(|b| b.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err("Invalid release version.");
    }
    value.parse().map_err(|_| "Invalid release version.")
}
fn newer_stable(candidate: &str, installed: &str) -> Result<bool, &'static str> {
    let (base, development) = match installed.split_once("-dev.") {
        Some((base, sequence)) if numeric(sequence)? > 0 => (base, true),
        Some(_) => return Err("Invalid installed version."),
        None => (installed, false),
    };
    let candidate = stable_version(candidate)?;
    let installed = stable_version(base)?;
    Ok(candidate > installed || (candidate == installed && development))
}
fn download_url(release: &Release) -> Result<String, &'static str> {
    stable_version(&release.tag_name)?;
    if release.draft || release.prerelease {
        return Err("Only published stable releases are supported.");
    }
    let number = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name);
    let name = format!("ParsecWebTurn-v{number}-win64.zip");
    let mut matches = release.assets.iter().filter(|a| a.name == name);
    let asset = matches
        .next()
        .ok_or("The stable release has no Windows ZIP.")?;
    if matches.next().is_some() {
        return Err("The stable release has duplicate Windows ZIPs.");
    }
    let expected = format!("{REPOSITORY}/releases/download/{}/{name}", release.tag_name);
    if asset.browser_download_url != expected {
        return Err("The download link is outside the expected repository.");
    }
    Ok(expected)
}
fn metadata(client: &reqwest::blocking::Client, url: &str) -> Result<Vec<u8>, &'static str> {
    let response = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|_| "Cannot reach GitHub. The network may block update checks.")?;
    if !response.status().is_success() {
        return Err("GitHub update check failed, was blocked, or was rate limited.");
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_METADATA as u64)
    {
        return Err("Release metadata is too large.");
    }
    let mut bytes = Vec::new();
    response
        .take((MAX_METADATA + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "Update check was interrupted.")?;
    if bytes.len() > MAX_METADATA {
        return Err("Release metadata is too large.");
    }
    Ok(bytes)
}
fn select(bytes: &[u8], installed: &str) -> Result<Option<Available>, &'static str> {
    let release: Release =
        serde_json::from_slice(bytes).map_err(|_| "Invalid GitHub release response.")?;
    if release.draft || release.prerelease {
        return Err("Only published stable releases are supported.");
    }
    if !newer_stable(&release.tag_name, installed)? {
        return Ok(None);
    }
    let download_url = download_url(&release)?;
    Ok(Some(Available {
        version: release.tag_name,
        notes: release
            .body
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
            .take(12000)
            .collect(),
        download_url,
    }))
}
fn latest() -> Result<Option<Available>, &'static str> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("ParsecWebTurn/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Cannot initialize the update check client.")?;
    select(&metadata(&client, API)?, env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release(tag: &str) -> Release {
        Release {
            tag_name: tag.into(),
            draft: false,
            prerelease: false,
            body: Some("Release notes".into()),
            assets: vec![Asset {
                name: format!("ParsecWebTurn-{tag}-win64.zip"),
                browser_download_url: format!(
                    "{REPOSITORY}/releases/download/{tag}/ParsecWebTurn-{tag}-win64.zip"
                ),
            }],
        }
    }
    #[test]
    fn development_build_can_upgrade_to_matching_stable_without_downgrading() {
        assert!(newer_stable("v0.9.3", "0.9.3-dev.1").unwrap());
        assert!(newer_stable("v0.10.0", "0.9.9-dev.2").unwrap());
        assert!(!newer_stable("v0.7.0", "0.9.3-dev.1").unwrap());
        assert!(!newer_stable("v0.9.3", "0.9.3").unwrap());
        for invalid in [
            "v1.0.0-beta",
            "01.0.0",
            "1.2",
            "1.2.3.4",
            "../1.2.3",
            "18446744073709551616.0.0",
        ] {
            assert!(newer_stable(invalid, "0.9.3-dev.1").is_err());
        }
        for invalid in ["0.9.3-dev.0", "0.9.3-dev.01", "0.9.3-dev.x", "0.9.3-beta"] {
            assert!(newer_stable("v0.9.4", invalid).is_err());
        }
    }
    #[test]
    fn only_unique_published_stable_repository_zip_can_be_opened() {
        let mut r = release("v0.9.3");
        assert!(download_url(&r).is_ok());
        r.assets[0]
            .browser_download_url
            .push_str("?redirect=elsewhere");
        assert!(download_url(&r).is_err());
        r = release("v0.9.3");
        r.assets.push(Asset {
            name: r.assets[0].name.clone(),
            browser_download_url: r.assets[0].browser_download_url.clone(),
        });
        assert!(download_url(&r).is_err());
        r = release("v0.9.3");
        r.prerelease = true;
        assert!(download_url(&r).is_err());
        r.prerelease = false;
        r.draft = true;
        assert!(download_url(&r).is_err());
        r.draft = false;
        r.assets.clear();
        assert!(download_url(&r).is_err());
        let bytes = br#"{"tag_name":"v0.9.3","draft":false,"prerelease":false,"body":"Notes","assets":[{"name":"ParsecWebTurn-v0.9.3-win64.zip","browser_download_url":"https://github.com/vampywiz17/ParsecWebTurn/releases/download/v0.9.3/ParsecWebTurn-v0.9.3-win64.zip"}]}"#;
        assert_eq!(
            select(bytes, "0.9.3-dev.1").unwrap().unwrap().notes,
            "Notes"
        );
        assert!(select(bytes, "0.9.4-dev.1").unwrap().is_none());
        assert!(select(b"not json", "0.9.3-dev.1").is_err());
    }
    #[test]
    fn metadata_rejects_redirects_and_bounds_bodies_with_or_without_length() {
        use std::io::Write;
        for response in [
            b"HTTP/1.1 302 Found\r\nLocation: https://other.example/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 1048577\r\nConnection: close\r\n\r\n".to_vec(),
            [b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".as_slice(), &vec![b'x'; MAX_METADATA + 1]].concat(),
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut request = [0; 2048]; assert!(stream.read(&mut request).unwrap() > 0);
                let _ = stream.write_all(&response);
            });
            let client = reqwest::blocking::Client::builder().no_proxy().timeout(Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::none()).build().unwrap();
            assert!(metadata(&client, &url).is_err()); server.join().unwrap();
        }
    }
    #[test]
    fn checks_do_not_block_the_ui_or_duplicate_requests_and_notify_only_when_available() {
        let state = Arc::new(State::default());
        let (tx, rx) = std::sync::mpsc::channel();
        state.check_with(true, move || {
            rx.recv().unwrap();
            Ok(Some(Available {
                version: "v0.9.3".into(),
                notes: "Notes".into(),
                download_url: "unused fixture".into(),
            }))
        });
        assert_eq!(state.view().phase, Phase::Checking);
        state.check_with(false, || panic!("duplicate request"));
        tx.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while state.view().phase == Phase::Checking && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(state.view().phase, Phase::Available);
        assert!(state.take_notification());
        assert!(!state.take_notification());
        state.check_with(true, || Err("Network unavailable"));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while state.view().phase == Phase::Checking && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(state.view().phase, Phase::Error);
        assert!(state.view().release.is_none());
        assert!(!state.take_notification());
    }
}
