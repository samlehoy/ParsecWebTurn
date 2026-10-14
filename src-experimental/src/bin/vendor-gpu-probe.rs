fn main() -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        let result = parsec_native_experimental::gpu::probe()?;
        println!("{}", serde_json::to_string_pretty(&result)?);
        Ok(())
    }
    #[cfg(not(windows))]
    anyhow::bail!("D3D11 GPU probes require Windows")
}
