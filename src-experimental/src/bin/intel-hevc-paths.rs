fn main() -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        let report = parsec_native_experimental::intel_hevc::probe()?;
        let path = std::env::current_exe()?.with_file_name("intel-hevc-paths.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&report)?)?;
        println!("Report: {}", path.display());
        println!("{}", serde_json::to_string_pretty(&report)?);
        Ok(())
    }
    #[cfg(not(windows))]
    anyhow::bail!("Intel HEVC paths require Windows")
}
