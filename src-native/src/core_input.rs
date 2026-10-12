//! Read-only adapter for input services omitted by the pinned 150-104a core.
//! See docs/native-immersive-input.md for the audited ABI and version boundary.
use crate::memory::GuestMemory;
use anyhow::{ensure, Context, Result};

const CONFIG_HASH: u32 = 2_954_756;
const KEY: u32 = 2_123_041;
const NAME: &[u8] = b"client_immersive\0";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Mode {
    pub mouse: bool,
    pub keyboard: bool,
}
impl Mode {
    fn decode(value: u32) -> Option<Self> {
        Some(match value {
            0 => Self::default(),
            1 => Self {
                mouse: true,
                keyboard: true,
            },
            2 => Self {
                mouse: false,
                keyboard: true,
            },
            3 => Self {
                mouse: true,
                keyboard: false,
            },
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct State {
    pub mode: Mode,
    pub detached: bool,
    pub warp_disabled: bool,
}

fn add(base: u32, offset: u32) -> Result<u32> {
    base.checked_add(offset)
        .context("input ABI address overflow")
}

pub fn mode(memory: &GuestMemory) -> Result<Mode> {
    // The build verifies the complete core SHA-256. Also reject an uninitialized
    // or different layout here; never scan memory or guess a mode from UI pixels.
    ensure!(
        memory.read(KEY, NAME.len())? == NAME,
        "input ABI unavailable"
    );
    let hash = memory.u32(CONFIG_HASH)?;
    ensure!(hash != 0, "configuration not initialized");
    let count = memory.u32(hash)?;
    ensure!((1..=65536).contains(&count), "invalid configuration hash");
    let buckets = memory.u32(add(hash, 4)?)?;
    let djb2 = NAME[..NAME.len() - 1].iter().fold(5381u32, |h, b| {
        h.wrapping_mul(33).wrapping_add(u32::from(*b))
    });
    let bucket = add(buckets, (djb2 % count) * 8)?;
    let len = memory.u32(bucket)?;
    ensure!(len <= 64, "configuration bucket exceeds limit");
    let entries = memory.u32(add(bucket, 4)?)?;
    for index in 0..len {
        let entry = add(entries, index * 8)?;
        let key = memory.u32(entry)?;
        if key == 0 || memory.read(key, NAME.len())? != NAME {
            continue;
        }
        let setting = memory.u32(add(entry, 4)?)?;
        ensure!(setting != 0, "missing immersive setting");
        let bytes = memory.read(setting, 252)?;
        // Configuration can be edited by the GUI worker. Reject a changing
        // snapshot instead of applying a partially updated capture policy.
        ensure!(
            bytes == memory.read(setting, 252)?,
            "configuration changing"
        );
        let word = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        ensure!(
            memory.read(word(0), NAME.len())? == NAME && word(4) == 1,
            "unexpected immersive setting type"
        );
        let mut layer = 0;
        for candidate in 1..=6 {
            let present = bytes[84 + candidate * 24];
            ensure!(present <= 1, "invalid configuration layer");
            if present != 0 {
                layer = candidate;
            }
        }
        ensure!(memory.u32(CONFIG_HASH)? == hash, "configuration replaced");
        return Mode::decode(word(88 + layer * 24)).context("unknown immersive mode");
    }
    anyhow::bail!("immersive setting missing")
}

pub fn state(memory: &GuestMemory, app: u32, mode: Mode) -> Result<State> {
    ensure!(app != 0, "application not initialized");
    // MTY_AppCreate stores the main-app opaque argument at +884. Verify the
    // backlink before reading its Disable Cursor Warp flag.
    let main = memory.u32(add(app, 884)?)?;
    ensure!(
        main != 0 && memory.u32(add(main, 9376)?)? == app,
        "input context mismatch"
    );
    Ok(State {
        mode,
        detached: memory.u32(add(app, 48)?)? != 0,
        warp_disabled: memory.read(add(main, 9464)?, 1)?[0] != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state(memory: &GuestMemory, app: u32) -> Result<State> {
        super::state(memory, app, mode(memory)?)
    }
    fn fixture() -> GuestMemory {
        let engine = wasmtime::Engine::new(&crate::core_config::config()).unwrap();
        let memory = GuestMemory(
            wasmtime::SharedMemory::new(&engine, wasmtime::MemoryType::shared(48, 48)).unwrap(),
        );
        memory.write(KEY, NAME).unwrap();
        memory.set_u32(CONFIG_HASH, 100_000).unwrap();
        memory.set_u32(100_000, 1).unwrap();
        memory.set_u32(100_004, 100_100).unwrap();
        memory.set_u32(100_100, 2).unwrap();
        memory.set_u32(100_104, 100_200).unwrap();
        // Include a collision: lookup must match the key, not the first slot.
        memory.write(100_500, b"different_setting\0").unwrap();
        memory.set_u32(100_200, 100_500).unwrap();
        memory.set_u32(100_208, KEY).unwrap();
        memory.set_u32(100_212, 101_000).unwrap();
        memory.set_u32(101_000, KEY).unwrap();
        memory.set_u32(101_004, 1).unwrap();
        memory.set_u32(4096 + 884, 8192).unwrap();
        memory.set_u32(8192 + 9376, 4096).unwrap();
        memory
    }
    #[test]
    fn modes_temporary_override_detach_and_warp_are_independent() {
        let m = fixture();
        for (value, mouse, keyboard) in [
            (0, false, false),
            (1, true, true),
            (2, false, true),
            (3, true, false),
        ] {
            m.set_u32(101_000 + 88, value).unwrap();
            assert_eq!(state(&m, 4096).unwrap().mode, Mode { mouse, keyboard });
        }
        // Higher-priority temporary Off must override saved Mouse.
        m.write(101_000 + 84 + 3 * 24, &[1]).unwrap();
        m.set_u32(101_000 + 88 + 3 * 24, 0).unwrap();
        assert_eq!(mode(&m).unwrap(), Mode::default());
        m.set_u32(4096 + 48, 1).unwrap();
        m.write(8192 + 9464, &[1]).unwrap();
        let s = state(&m, 4096).unwrap();
        assert!(s.detached && s.warp_disabled);
        m.set_u32(8192 + 9376, 0).unwrap();
        assert!(state(&m, 4096).is_err());
    }
    #[test]
    fn invalid_layout_type_mode_and_bucket_fail_without_capture() {
        let m = fixture();
        m.set_u32(101_004, 2).unwrap();
        assert!(mode(&m).is_err());
        m.set_u32(101_004, 1).unwrap();
        m.set_u32(101_088, 4).unwrap();
        assert!(mode(&m).is_err());
        m.set_u32(100_100, u32::MAX).unwrap();
        assert!(mode(&m).is_err());
        m.write(KEY, &[0]).unwrap();
        assert!(mode(&m).is_err());
    }
    #[test]
    fn pinned_core_initializes_the_audited_immersive_schema() {
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                let engine = wasmtime::Engine::new(&crate::core_config::config()).unwrap();
                let module = crate::core_runtime::load(&engine).unwrap();
                let (mut store, instance) = crate::instantiate(&engine, &module).unwrap();
                // Offline bootstrap stops at the unsupported graphics boundary;
                // configuration initialization preceding it is the real pinned code.
                let _ = instance
                    .get_typed_func::<(), ()>(&mut store, "_start")
                    .unwrap()
                    .call(&mut store, ());
                assert_eq!(mode(&store.data().memory).unwrap(), Mode::default());
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
