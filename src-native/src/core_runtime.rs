//! Load only the immutable native-code artifact produced by our own build.
pub fn load(engine: &wasmtime::Engine) -> anyhow::Result<wasmtime::Module> {
    use anyhow::Context;
    // SAFETY: build.rs verifies the original WASM hash and compiles this embedded
    // artifact with the same locked Wasmtime version and shared execution config.
    // No external serialized code, writable cache or user-supplied path is used.
    unsafe {
        wasmtime::Module::deserialize(
            engine,
            include_bytes!(concat!(env!("OUT_DIR"), "/parsecd.cwasm")),
        )
    }
    .context("Loading the embedded precompiled Parsec core")
}

#[cfg(test)]
mod tests {
    #[test]
    fn embedded_core_executes_with_fuel_and_epoch_limits() {
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                let engine = wasmtime::Engine::new(&crate::core_config::config()).unwrap();
                let module = super::load(&engine).unwrap();
                let (mut store, instance) = crate::instantiate(&engine, &module).unwrap();
                crate::allocator_roundtrip(&mut store, &instance).unwrap();
                let alloc = instance
                    .get_typed_func::<(i32, i32), i32>(&mut store, "mty_system_alloc")
                    .unwrap();
                store.set_fuel(0).unwrap();
                let error = alloc.call(&mut store, (32, 1)).unwrap_err();
                assert_eq!(
                    error.downcast_ref::<wasmtime::Trap>(),
                    Some(&wasmtime::Trap::OutOfFuel)
                );
                store.set_fuel(50_000_000).unwrap();
                store.set_epoch_deadline(1);
                engine.increment_epoch();
                let error = alloc.call(&mut store, (32, 1)).unwrap_err();
                assert_eq!(
                    error.downcast_ref::<wasmtime::Trap>(),
                    Some(&wasmtime::Trap::Interrupt)
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
