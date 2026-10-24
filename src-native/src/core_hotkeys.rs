//! Read-only conflict arbitration for the unchanged, SHA-pinned 150-104a core.
//! The core itself executes and persists hotkeys; Rust only yields its shortcuts.
use crate::memory::GuestMemory;
use anyhow::{ensure, Context, Result};
use std::collections::BTreeMap;

pub struct Binding {
    pub memory: GuestMemory,
    pub app: u32,
    pub keys: BTreeMap<String, i32>,
}
impl Binding {
    pub fn owns(&self, code: &str, mods: i32) -> Result<bool> {
        let Some(key) = self.keys.get(code) else {
            return Ok(false);
        };
        let modifiers = (mods & 1) | ((mods << 1) & 4) | ((mods << 2) & 16) | ((mods << 3) & 64);
        let target = (modifiers as u32) << 16 | *key as u32;
        let at =
            |base: u32, offset: u32| base.checked_add(offset).context("hotkey address overflow");
        let slot = at(self.app, 32)?;
        let hash = self.memory.u32(slot)?;
        ensure!(hash != 0, "hotkey hash unavailable");
        let header = self.memory.read(hash, 8)?;
        let count = u32::from_le_bytes(header[..4].try_into()?);
        let buckets = u32::from_le_bytes(header[4..].try_into()?);
        ensure!((1..=1024).contains(&count), "invalid hotkey hash");
        let mut any = false;
        let mut matched = false;
        let mut total = 0;
        for index in 0..count {
            let bucket = at(buckets, index * 8)?;
            let bytes = self.memory.read(bucket, 8)?;
            let length = u32::from_le_bytes(bytes[..4].try_into()?);
            ensure!(length <= 64, "invalid hotkey bucket");
            total += length;
            ensure!(total <= 64, "hotkey hash exceeds limit");
            let entries = u32::from_le_bytes(bytes[4..].try_into()?);
            for i in 0..length {
                let entry = at(entries, i * 8)?;
                let name = self.memory.u32(entry)?;
                if name == 0 {
                    continue;
                }
                let id = self.memory.u32(at(entry, 4)?)?;
                ensure!((1..=13).contains(&id), "invalid hotkey action");
                // MTY_HashSetInt/GetInt format the integer as "#%llx".
                let text = self.memory.string(name, 32)?;
                let value =
                    u32::from_str_radix(text.strip_prefix('#').context("invalid hotkey key")?, 16)?;
                any = true;
                matched |= value == target;
            }
            ensure!(
                bytes == self.memory.read(bucket, 8)?,
                "hotkey hash changing"
            );
        }
        ensure!(
            header == self.memory.read(hash, 8)? && self.memory.u32(slot)? == hash,
            "hotkey hash replaced"
        );
        // Recording temporarily removes the core's hotkeys. Let the core see
        // every key then, including F8/F11 and native settings/stats shortcuts.
        Ok(matched || !any)
    }
}

#[cfg(all(test, not(feature = "native-core")))]
mod tests {
    use super::*;
    #[test]
    #[cfg(not(feature = "native-core"))]
    fn original_core_dispatches_custom_hotkeys_and_consumes_their_release() {
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                let engine = wasmtime::Engine::new(&crate::core_config::config()).unwrap();
                let module = crate::core_runtime::load(&engine).unwrap();
                let (mut store, instance) = crate::instantiate(&engine, &module).unwrap();
                // The offline watchdog advances this isolated engine once.
                // Host-side fixture setup can outlast that timer on CI; keep
                // the finite fuel budget as the bound for these guest calls.
                store.set_epoch_deadline(2);
                let _ = instance
                    .get_typed_func::<(), ()>(&mut store, "_start")
                    .unwrap()
                    .call(&mut store, ());
                instance
                    .get_typed_func::<(), ()>(&mut store, "mty_app_set_keys")
                    .unwrap()
                    .call(&mut store, ())
                    .unwrap();
                let alloc = instance
                    .get_typed_func::<(i32, i32), i32>(&mut store, "mty_system_alloc")
                    .unwrap();
                let app = alloc.call(&mut store, (4096, 1)).unwrap() as u32;
                let memory = store.data().memory.clone();
                memory.write(app, &vec![0; 4096]).unwrap();
                let key = store.data().key_codes["F11"];
                let hash = app + 1024;
                let buckets = app + 1040;
                let entries = app + 1056;
                let name = app + 1072;
                memory.set_u32(app + 32, hash).unwrap();
                memory.set_u32(hash, 1).unwrap();
                memory.set_u32(hash + 4, buckets).unwrap();
                memory.set_u32(buckets, 1).unwrap();
                memory.set_u32(buckets + 4, entries).unwrap();
                memory.set_u32(entries, name).unwrap();
                memory.set_u32(entries + 4, 2).unwrap(); // fullscreen action
                memory
                    .c_string(name, 32, &format!("#{:x}", (21u32 << 16) | key as u32))
                    .unwrap(); // Shift+Ctrl+Alt
                let binding = Binding {
                    memory: memory.clone(),
                    app,
                    keys: store.data().key_codes.clone(),
                };
                assert!(binding.owns("F11", 7).unwrap());
                assert!(binding.owns("F11", 7 | 48).unwrap()); // lock states ignored
                assert!(!binding.owns("F11", 0).unwrap());
                assert!(!binding.owns("F8", 7).unwrap());
                let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
                let events = captured.clone();
                let callback = wasmtime::Func::wrap(
                    &mut store,
                    move |caller: wasmtime::Caller<'_, crate::host::HostState>,
                          event: i32,
                          _: i32| {
                        let m = &caller.data().memory;
                        if m.u32(event as u32).unwrap() == 16 {
                            assert!(caller.data().clipboard_notification);
                            assert!(m
                                .read(event as u32 + 4, 212)
                                .unwrap()
                                .iter()
                                .all(|b| *b == 0));
                        }
                        events.lock().unwrap().push((
                            m.u32(event as u32).unwrap(),
                            m.u32(event as u32 + 8).unwrap(),
                        ));
                    },
                );
                let table = instance
                    .get_table(&mut store, "__indirect_function_table")
                    .unwrap();
                // Slot zero is unused by this isolated fixture. The original
                // fixed-size table is not grown or rewritten in production.
                let index = 0;
                table
                    .set(&mut store, index, wasmtime::Ref::Func(Some(callback)))
                    .unwrap();
                memory.set_u32(app + 40, index as u32).unwrap();
                let keyboard = instance
                    .get_typed_func::<(i32, i32, i32, i32, i32), ()>(
                        &mut store,
                        "mty_window_keyboard",
                    )
                    .unwrap();
                keyboard
                    .call(&mut store, (app as i32, 1, key, 0, 7))
                    .unwrap();
                keyboard
                    .call(&mut store, (app as i32, 0, key, 0, 7))
                    .unwrap();
                keyboard
                    .call(&mut store, (app as i32, 1, key, 0, 0))
                    .unwrap();
                crate::desktop::dispatch_clipboard_event(&mut store, &instance, app).unwrap();
                assert!(!store.data().clipboard_notification);
                assert_eq!(
                    *captured.lock().unwrap(),
                    [(6, 2), (0, key as u32), (5, key as u32), (16, 0)]
                );
                // Rebinding and recording take effect immediately, without a Rust cache.
                memory.set_u32(buckets, 0).unwrap();
                assert!(binding.owns("F8", 0).unwrap());
                memory.set_u32(hash, u32::MAX).unwrap();
                assert!(binding.owns("F11", 0).is_err());
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
