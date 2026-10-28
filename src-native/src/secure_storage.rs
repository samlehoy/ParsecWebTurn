//! Protected storage: Windows CurrentUser DPAPI on Windows; ChaCha20-Poly1305 with local user key on macOS/Unix.
use anyhow::{bail, Result};

#[cfg(windows)]
use windows_sys::Win32::{Foundation::LocalFree, Security::Cryptography::*};

#[cfg(windows)]
pub fn protect(input: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    let blob = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(input.len())?,
        pbData: input.as_ptr() as *mut u8,
    };
    let mut output: CRYPT_INTEGER_BLOB = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        if encrypt {
            CryptProtectData(
                &blob,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &blob,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if ok == 0 {
        bail!("Windows profile protection failed");
    }
    // DPAPI allocates this buffer; copy before releasing with the documented allocator.
    let bytes = if output.cbData == 0 {
        Vec::new()
    } else if output.pbData.is_null() {
        bail!("Windows profile protection returned no buffer");
    } else {
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() }
    };
    unsafe {
        LocalFree(output.pbData.cast());
    }
    Ok(bytes)
}

#[cfg(not(windows))]
pub fn protect(input: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    use ring::aead::{
        Aad, BoundKey, Nonce, NonceSequence, OpeningKey, SealingKey, UnboundKey, CHACHA20_POLY1305,
        NONCE_LEN,
    };
    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::PathBuf;

    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    let key_dir = home.join("Library/Application Support/ParsecWebTurn");
    let _ = fs::create_dir_all(&key_dir);
    let key_path = key_dir.join(".profile_key");

    let key_bytes = match fs::read(&key_path) {
        Ok(key) => key,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut key = vec![0u8; 32];
            getrandom::fill(&mut key)
                .map_err(|e| anyhow::anyhow!("Random key generation failed: {e}"))?;
            // Publish only a complete, synced key: a crash cannot leave an empty key file,
            // and a concurrent first run that links first wins; we then use its key.
            let temporary = key_dir.join(format!(".profile_key.{}.tmp", std::process::id()));
            let _ = fs::remove_file(&temporary);
            let created = (|| {
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&temporary)?;
                file.write_all(&key)?;
                file.sync_all()?;
                fs::hard_link(&temporary, &key_path)
            })();
            let _ = fs::remove_file(&temporary);
            match created {
                Ok(()) => {
                    fs::File::open(&key_dir)?.sync_all()?;
                    key
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => fs::read(&key_path)?,
                Err(e) => return Err(e.into()),
            }
        }
        Err(e) => return Err(e.into()),
    };

    if key_bytes.len() != 32 {
        bail!("Invalid profile key length");
    }

    struct OneNonce(Option<[u8; NONCE_LEN]>);
    impl NonceSequence for OneNonce {
        fn advance(&mut self) -> std::result::Result<Nonce, ring::error::Unspecified> {
            self.0
                .take()
                .map(Nonce::assume_unique_for_key)
                .ok_or(ring::error::Unspecified)
        }
    }

    let unbound = UnboundKey::new(&CHACHA20_POLY1305, &key_bytes)
        .map_err(|_| anyhow::anyhow!("Cannot create cipher key"))?;

    if encrypt {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce_bytes)
            .map_err(|e| anyhow::anyhow!("Nonce generation failed: {e}"))?;
        let mut sealing_key = SealingKey::new(unbound, OneNonce(Some(nonce_bytes)));
        let mut in_out = input.to_vec();
        sealing_key
            .seal_in_place_append_tag(Aad::empty(), &mut in_out)
            .map_err(|_| anyhow::anyhow!("Encryption failed"))?;
        let mut result = nonce_bytes.to_vec();
        result.extend_from_slice(&in_out);
        Ok(result)
    } else {
        if input.len() < NONCE_LEN + CHACHA20_POLY1305.tag_len() {
            bail!("Input too short for decryption");
        }
        let (nonce_slice, ciphertext) = input.split_at(NONCE_LEN);
        let mut nonce_bytes = [0u8; NONCE_LEN];
        nonce_bytes.copy_from_slice(nonce_slice);
        let mut opening_key = OpeningKey::new(unbound, OneNonce(Some(nonce_bytes)));
        let mut in_out = ciphertext.to_vec();
        let plain = opening_key
            .open_in_place(Aad::empty(), &mut in_out)
            .map_err(|_| anyhow::anyhow!("Decryption failed; profile signature invalid"))?;
        Ok(plain.to_vec())
    }
}
