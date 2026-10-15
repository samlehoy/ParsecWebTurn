fn main() -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        let report = parsec_native_experimental::gpu::intel_hevc_compare()?;
        std::fs::write(
            "intel-hevc-comparison.json",
            serde_json::to_vec_pretty(&report)?,
        )?;
        println!("Saved intel-hevc-comparison.json");
        Ok(())
    }
    #[cfg(not(windows))]
    anyhow::bail!("Intel HEVC comparison requires Windows")
}
