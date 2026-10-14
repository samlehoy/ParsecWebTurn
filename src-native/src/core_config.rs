// Shared by the AOT build and runtime loader. Keep all
// execution/safety settings identical; AOT changes loading, not guest policy.
pub fn config() -> wasmtime::Config {
    let mut config = wasmtime::Config::new();
    config
        .wasm_threads(true)
        .consume_fuel(true)
        .epoch_interruption(true);
    config
}
