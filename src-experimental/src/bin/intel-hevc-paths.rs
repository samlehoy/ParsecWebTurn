fn main() -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        let wait_for_trace = std::env::args().any(|arg| arg == "--wait-for-trace");
        if wait_for_trace {
            // Keep codec discovery in the ordinary user's context. Only the
            // external, documented Microsoft tracing tool needs elevation.
            let helper =
                std::env::current_exe()?.with_file_name("Attach-Intel-HEVC-Trace-as-admin.cmd");
            let script = include_str!("../../Attach-Intel-HEVC-Trace.cmd.in")
                .replace("@PROBE_PID@", &std::process::id().to_string());
            std::fs::write(&helper, script.replace("\r\n", "\n").replace('\n', "\r\n"))?;
            println!("Test PID: {}. Keep this window open.", std::process::id());
            println!(
                "Right-click {} and run it as administrator.",
                helper.display()
            );
            println!("After MFTrace has attached, return here and press Enter.");
            let mut line = String::new();
            anyhow::ensure!(
                std::io::stdin().read_line(&mut line)? > 0,
                "trace preparation needs an interactive console"
            );
        }
        let report = parsec_native_experimental::intel_hevc::probe()?;
        let path = std::env::current_exe()?.with_file_name("intel-hevc-paths.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&report)?)?;
        if wait_for_trace {
            std::fs::write(
                path.with_file_name("intel-hevc-console.txt"),
                serde_json::to_vec_pretty(&report)?,
            )?;
        }
        println!("Report: {}", path.display());
        println!("{}", serde_json::to_string_pretty(&report)?);
        Ok(())
    }
    #[cfg(not(windows))]
    anyhow::bail!("Intel HEVC paths require Windows")
}
