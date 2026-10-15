fn main() -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        let gpu_trace = std::env::args().any(|arg| arg == "--wait-for-gpu-trace");
        let wait_for_trace = gpu_trace || std::env::args().any(|arg| arg == "--wait-for-trace");
        if wait_for_trace {
            // Keep codec discovery in the ordinary user's context. Only the
            // external, documented Microsoft tracing tool needs elevation.
            let (name, template, ready) = if gpu_trace {
                (
                    "Start-Intel-HEVC-GPU-Trace-as-admin.cmd",
                    include_str!("../../Start-Intel-HEVC-GPU-Trace.cmd.in"),
                    "After WPR says recording started, return here and press Enter.",
                )
            } else {
                (
                    "Attach-Intel-HEVC-Trace-as-admin.cmd",
                    include_str!("../../Attach-Intel-HEVC-Trace.cmd.in"),
                    "After MFTrace has attached, return here and press Enter.",
                )
            };
            let helper = std::env::current_exe()?.with_file_name(name);
            let pid = std::process::id().to_string();
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis();
            let instance = format!("PWTIntelHevc_{pid}_{stamp}");
            let script = template
                .replace("@PROBE_PID@", &pid)
                .replace("@TRACE_INSTANCE@", &instance);
            std::fs::write(&helper, script.replace("\r\n", "\n").replace('\n', "\r\n"))?;
            if gpu_trace {
                let stop = include_str!("../../Stop-Intel-HEVC-GPU-Trace.cmd.in")
                    .replace("@TRACE_INSTANCE@", &instance);
                std::fs::write(
                    helper.with_file_name("Stop-Intel-HEVC-GPU-Trace-as-admin.cmd"),
                    stop.replace("\r\n", "\n").replace('\n', "\r\n"),
                )?;
                std::fs::write(
                    helper.with_file_name("trace-target.txt"),
                    format!("Test PID: {pid}\r\nWPR instance: {instance}\r\n"),
                )?;
            }
            println!("Test PID: {}. Keep this window open.", std::process::id());
            println!(
                "Right-click {} and run it as administrator.",
                helper.display()
            );
            println!("{ready}");
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
