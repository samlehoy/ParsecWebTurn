//! Optional vendor runtime installation. Never called from the video worker.
//! Only the exact pinned NVIDIA archive is accepted. Its two needed entries
//! are fixed deflate streams; this is deliberately not a general ZIP extractor.
use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::Duration,
};

pub const DLL_HASH: &str = "c3d88eea5ff7a548edefa66414cf6e77464d0947277c904f324dd23abf58a1ed";
const ARCHIVE_HASH: &str = "abf4f34e2b5a618e355b0d5a0365d8ecc3db4396e756e4c850a867e1ae2ed69e";
const ARCHIVE_SIZE: u64 = 63_493_768;
const DLL_SIZE: u64 = 19_140_144;
const URL: &str = "https://api.ngc.nvidia.com/v2/models/nvidia/multimedia/dlpp/versions/1.5/files/RTX_Video_SDK_v1.1.0.zip";
pub const PROMPT: &str = "Download the optional NVIDIA RTX Video Super Resolution runtime from NVIDIA?\n\nThe download is about 61 MB. The checked NVIDIA DLL (about 19 MB) will be stored in a VSR subfolder beside settings.json and the encrypted sign-in data. Sign-in opens after setup finishes.\n\nVSR requires a compatible RTX GPU/driver, is off by default and may increase latency. The NVIDIA SDK license is included in the application package. Choosing No keeps ordinary GPU video available.";
pub const RESTART_HINT: &str =
    " Restart the app to download the approved NVIDIA VSR runtime before sign-in.";

pub fn directory(data: &Path) -> PathBuf {
    data.join("vsr").join("1.1.0")
}
pub fn present(data: &Path) -> bool {
    directory(data).join("nvngx_vsr.dll").is_file()
}
fn hash(mut file: &File) -> Result<String> {
    let mut digest = Sha256::new();
    std::io::copy(&mut file, &mut digest)?;
    Ok(format!("{:x}", digest.finalize()))
}
fn verify(path: &Path, size: u64, expected: &str) -> Result<()> {
    let file = File::open(path)?;
    ensure!(
        file.metadata()?.len() == size && hash(&file)? == expected,
        "NVIDIA runtime checksum mismatch"
    );
    Ok(())
}
pub fn validated_directory(data: &Path) -> Result<PathBuf> {
    let root = directory(data);
    verify(&root.join("nvngx_vsr.dll"), DLL_SIZE, DLL_HASH)
        .context("VSR runtime missing or invalid; approve its download in Settings and restart")?;
    Ok(std::fs::canonicalize(root)?)
}
/// Reuse only our previously pinned runtime, not arbitrary driver/application DLLs.
pub fn reuse_previous(data: &Path) -> Result<bool> {
    if present(data) {
        return Ok(true);
    }
    let Some(local) = std::env::var_os("LOCALAPPDATA") else {
        return Ok(false);
    };
    let old = PathBuf::from(local)
        .join("EvoParseq/rtx-video")
        .join(DLL_HASH)
        .join("nvngx_vsr.dll");
    if !old.is_file() {
        return Ok(false);
    }
    verify(&old, DLL_SIZE, DLL_HASH)?;
    let root = directory(data);
    std::fs::create_dir_all(&root)?;
    let temporary = root.join("runtime.tmp");
    std::fs::copy(old, &temporary)?;
    verify(&temporary, DLL_SIZE, DLL_HASH)?;
    std::fs::rename(&temporary, root.join("nvngx_vsr.dll"))?;
    Ok(true)
}
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub fn install(data: &Path) -> Result<()> {
    let root = directory(data);
    std::fs::create_dir_all(&root)?;
    let temporary = Temporary(root.join("sdk-download.tmp"));
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.url().scheme() != "https" || attempt.previous().len() > 5 {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()?;
    let response = client
        .get(URL)
        .send()
        .context("Cannot contact NVIDIA download service")?
        .error_for_status()
        .context("NVIDIA download service rejected the request")?;
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .context("NVIDIA download location missing")?
        .to_str()?;
    let location = reqwest::Url::parse(location)?;
    ensure!(
        location.scheme() == "https",
        "NVIDIA download must use HTTPS"
    );
    let response = client
        .get(location)
        .send()
        .map_err(|_| anyhow::anyhow!("NVIDIA download failed"))?
        .error_for_status()
        .map_err(|_| anyhow::anyhow!("NVIDIA download was rejected"))?;
    let mut archive = File::create(&temporary.0)?;
    let count = std::io::copy(&mut response.take(ARCHIVE_SIZE + 1), &mut archive)?;
    drop(archive);
    ensure!(count == ARCHIVE_SIZE, "NVIDIA SDK download size mismatch");
    verify(&temporary.0, ARCHIVE_SIZE, ARCHIVE_HASH)?;
    // These offsets are valid ONLY after checking the complete immutable SDK
    // 1.1.0 archive hash above. No filenames from an archive become filesystem paths.
    extract(
        &temporary.0,
        &root.join("NVIDIA-RTX-VIDEO-LICENSE.pdf"),
        62_433_254,
        131_214,
        136_535,
    )?;
    let dll = Temporary(root.join("runtime.tmp"));
    extract(&temporary.0, &dll.0, 37_964_664, 14_455_982, DLL_SIZE)?;
    verify(&dll.0, DLL_SIZE, DLL_HASH)?;
    std::fs::rename(&dll.0, root.join("nvngx_vsr.dll"))?;
    crate::event_log::record(
        "INFO",
        "vsr_runtime_installed",
        serde_json::json!({"sdk_version":"1.1.0","sha256":DLL_HASH}),
    );
    Ok(())
}
fn extract(archive: &Path, output: &Path, offset: u64, compressed: u64, size: u64) -> Result<()> {
    let mut input = File::open(archive)?;
    input.seek(SeekFrom::Start(offset))?;
    let decoder = flate2::read::DeflateDecoder::new(input.take(compressed));
    let mut file = File::create(output)?;
    ensure!(
        std::io::copy(&mut decoder.take(size + 1), &mut file)? == size,
        "NVIDIA runtime extraction size mismatch"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mismatched_vendor_runtime_is_rejected_before_loading() {
        let path =
            std::env::temp_dir().join(format!("evoparseq-vsr-corrupt-{}", std::process::id()));
        std::fs::write(&path, b"not NVIDIA code").unwrap();
        assert!(verify(&path, 15, DLL_HASH).is_err());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn pinned_archive_stream_offsets_reproduce_the_official_runtime() {
        let Some(sdk) = std::env::var_os("EVOPARSEQ_RTX_VIDEO_SDK") else {
            return;
        };
        let archive = PathBuf::from(sdk)
            .parent()
            .unwrap()
            .join("RTX_Video_SDK_v1.1.0.zip");
        verify(&archive, ARCHIVE_SIZE, ARCHIVE_HASH).unwrap();
        let path =
            std::env::temp_dir().join(format!("evoparseq-vsr-extract-{}", std::process::id()));
        extract(&archive, &path, 37_964_664, 14_455_982, DLL_SIZE).unwrap();
        verify(&path, DLL_SIZE, DLL_HASH).unwrap();
        std::fs::remove_file(path).unwrap();
    }
}
