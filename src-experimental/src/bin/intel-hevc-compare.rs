fn main() -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        let report = parsec_native_experimental::gpu::intel_hevc_compare()?;
        let output = if std::env::args().any(|arg| arg == "--presentation-only") {
            "intel-presentation-diagnostics.json"
        } else {
            "intel-hevc-comparison.json"
        };
        std::fs::write(output, serde_json::to_vec_pretty(&report)?)?;
        println!("Saved {output}");
        Ok(())
    }
    #[cfg(not(windows))]
    anyhow::bail!("Intel HEVC comparison requires Windows")
}
