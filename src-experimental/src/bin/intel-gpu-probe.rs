fn main() -> anyhow::Result<()> {
    #[cfg(windows)]
    println!(
        "{}",
        serde_json::to_string_pretty(&parsec_native_experimental::intel::probe()?)?
    );
    #[cfg(not(windows))]
    anyhow::bail!("Intel D3D11 probe requires Windows");
    #[cfg(windows)]
    Ok(())
}
