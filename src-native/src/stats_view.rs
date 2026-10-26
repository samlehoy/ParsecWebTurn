//! Presentation of existing telemetry; no sampling, window APIs or video work.
use crate::stats::{MemoryUsage, Sample, Values};

/// Empty values denote section headings; all units/text are decided here.
pub type Row = (String, String);
pub struct Presentation<'a> {
    pub status: &'a str,
    pub cards: Vec<(&'static str, String)>,
    pub rows: Vec<Row>,
    pub footer: String,
}
impl<'a> Presentation<'a> {
    pub fn new(
        sample: &'a Sample,
        values: &Values,
        usage: (Option<f64>, Option<f64>, Option<f64>),
        memory: Option<MemoryUsage>,
        timing: &crate::frame_timing::Snapshot,
    ) -> Self {
        let stale = sample.at.elapsed().as_secs() > 4 && sample.state == "connected";
        let status = if stale {
            "Waiting for fresh telemetry"
        } else {
            &sample.state
        };
        let cards = [
            ("Connection RTT", number(sample.rtt_ms, "ms")),
            ("Decoded video", number(values.fps, "fps")),
            ("Incoming traffic", number(values.incoming, "Mbps")),
            ("Outgoing traffic", number(values.outgoing, "Mbps")),
        ];
        #[cfg(feature = "bud")]
        let mut cards: Vec<_> = if sample.bud.is_some() {
            cards
                .into_iter()
                .filter(|(_, value)| value != "—")
                .collect()
        } else {
            cards.into()
        };
        #[cfg(not(feature = "bud"))]
        let mut cards: Vec<_> = cards.into();
        // Keep the existing policy: BUD hides unavailable cards, then stale
        // telemetry replaces values in the remaining cards with an em dash.
        if stale {
            for (_, value) in &mut cards {
                value.clear();
                value.push('—');
            }
        }
        let mut rows = rows(sample, values, usage);
        if let Some(memory) = memory {
            for (label, bytes) in [
                ("Memory resident (total)", Some(memory.working_set_bytes)),
                (
                    "Memory resident (private)",
                    memory.private_working_set_bytes,
                ),
                (
                    "Memory committed (private)",
                    Some(memory.private_committed_bytes),
                ),
            ] {
                if let Some(bytes) = bytes {
                    rows.push((
                        label.into(),
                        format!("{:.1} MiB", bytes as f64 / 1_048_576.),
                    ));
                }
            }
        }
        rows.extend(host_timing_rows(timing));
        #[cfg(feature = "bud")]
        let footer = if sample.bud.is_some() {
            "Traffic counts UDP payload bytes or TURN stream plaintext accepted for send or received, including BUD/crypto/TURN control and retransmissions. Stream framing padding is included; TLS, IP and TCP/UDP headers and account HTTPS/WSS traffic are excluded. RTT is the native transport estimate. Host encode time keeps the last available report. Late fragments indicate reordering or retransmission, not proven packet loss. NACKs request missing fragments. Compressed backlog is before decoding; no GPU timings are inferred."
        } else {
            WEBRTC_FOOTER
        };
        #[cfg(not(feature = "bud"))]
        let footer = WEBRTC_FOOTER;
        let footer = if timing.requested {
            format!("{footer} Host timing averages and peaks cover the current diagnostic collection. These are host durations, not capture-to-display latency or GPU execution times.")
        } else {
            footer.to_owned()
        };
        Self {
            status,
            cards,
            rows,
            footer,
        }
    }
}
const WEBRTC_FOOTER: &str = "Traffic is measured usage, not available bandwidth. RTT is the active ICE path's round-trip time, not ICMP ping. Local dropped frames are not network packet loss. Missing values remain unknown.";

fn number(value: Option<f64>, unit: &str) -> String {
    value
        .filter(|v| v.is_finite())
        .map(|v| format!("{v:.1} {unit}"))
        .unwrap_or_else(|| "—".into())
}
fn reported(s: &str) -> String {
    if s.is_empty() {
        "Not reported".into()
    } else {
        s.into()
    }
}
fn host_timing_rows(s: &crate::frame_timing::Snapshot) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    if !s.requested && !s.request_failed {
        return rows;
    }
    rows.push(("HOST TIMING".into(), String::new()));
    if s.request_failed {
        rows.push(("Host timing".into(), "Unavailable (request failed)".into()));
        return rows;
    }
    rows.push((
        "Timing values".into(),
        "Latest / average / peak (ms)".into(),
    ));
    let value = |series: &crate::frame_timing::Series, i: usize| {
        let n = |v: f64| {
            if v < 1_000_000. {
                format!("{v:.2}")
            } else {
                format!("{v:.2e}")
            }
        };
        format!(
            "{} / {} / {}",
            n(series.last[i]),
            n(series.mean[i]),
            n(series.peak[i])
        )
    };
    let age = |series: &crate::frame_timing::Series| {
        let seconds = series.at.elapsed().as_secs_f64();
        if seconds > 3. {
            format!("Stale ({seconds:.1} s old)")
        } else {
            format!("{seconds:.1} s")
        }
    };
    if s.video.iter().all(Option::is_none) {
        rows.push(("Video timing".into(), "Not reported by host".into()));
    }
    for (stream, series) in s.video.iter().enumerate() {
        let Some(series) = series else {
            continue;
        };
        let prefix = if stream == 0 {
            String::new()
        } else {
            format!("Stream {stream} ")
        };
        for (i, label) in [
            "Video processing",
            "Capture segment",
            "Host encode",
            "Video frame interval",
        ]
        .into_iter()
        .enumerate()
        {
            rows.push((format!("{prefix}{label}"), value(series, i)));
        }
        rows.push((
            format!("{prefix}Video timing reports"),
            series.count.to_string(),
        ));
        rows.push((format!("{prefix}Video report age"), age(series)));
    }
    if let Some(series) = &s.audio {
        rows.push(("Audio processing".into(), value(series, 0)));
        rows.push(("Audio block interval".into(), value(series, 2)));
        // The intermediate host slot is not a complete per-peer codec timing.
        rows.push(("Audio timing reports".into(), series.count.to_string()));
        rows.push(("Audio report age".into(), age(series)));
    } else {
        rows.push(("Audio timing".into(), "Not reported by host".into()));
    }
    if s.rejected > 0 {
        rows.push(("Invalid timing reports".into(), s.rejected.to_string()));
    }
    if s.unknown > 0 {
        rows.push(("Unknown timing formats".into(), s.unknown.to_string()));
    }
    rows
}
fn webrtc_network_rows(s: &Sample, row: &mut impl FnMut(&str, String)) {
    row("NETWORK", String::new());
    row(
        "Route",
        if s.state == "connected" && s.at.elapsed().as_secs() > 4 {
            "Unverified (telemetry unavailable)".into()
        } else {
            s.route.into()
        },
    );
    row("Local candidate", reported(&s.local));
    row("Remote candidate", reported(&s.remote));
    row("Transport", reported(&s.protocol));
    row(
        "TURN server in use",
        s.turn_server.clone().unwrap_or_else(|| {
            if s.route == "Direct — no TURN" {
                "None".into()
            } else {
                "Not reported".into()
            }
        }),
    );
    row(
        "TURN transport",
        s.turn_protocol
            .clone()
            .unwrap_or_else(|| "Not reported".into()),
    );
}
fn webrtc_security_rows(s: &Sample, row: &mut impl FnMut(&str, String)) {
    row("SECURITY", String::new());
    row("DTLS state", s.dtls.clone());
    row(
        "Encryption",
        if s.dtls == "connected" {
            "DTLS protects SCTP data channels".into()
        } else {
            "Not reported".into()
        },
    );
}
fn rows(
    s: &Sample,
    v: &Values,
    cpu: (Option<f64>, Option<f64>, Option<f64>),
) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let mut row = |a: &str, b: String| rows.push((a.into(), b));
    #[cfg(feature = "bud")]
    if let Some(bud) = &s.bud {
        row("NETWORK", String::new());
        row("Transport", "BUD / UDP".into());
        if bud.connected {
            row(
                "Route",
                if bud.relay { "TURN relay" } else { "Direct" }.into(),
            );
        }
        if let Some(carrier) = bud.relay_transport.filter(|_| bud.relay) {
            row("TURN transport", carrier.to_uppercase());
        }
        row(
            "Host encode time",
            bud.encode_ms
                .map(|encode| number(Some(encode), "ms"))
                .unwrap_or_else(|| "Waiting for host".into()),
        );
        row("BUD DELIVERY", String::new());
        for (name, channel) in ["Control", "Video", "Audio"].iter().zip(&bud.channels) {
            row(&format!("{name} late fragments"), channel.late.to_string());
            row(&format!("{name} NACK requests"), channel.nacks.to_string());
        }
        row(
            "Duplicate fragments",
            bud.channels
                .iter()
                .map(|c| c.duplicates)
                .sum::<u64>()
                .to_string(),
        );
        row(
            "Outside receive window",
            bud.channels
                .iter()
                .map(|c| c.window_drops)
                .sum::<u64>()
                .to_string(),
        );
        row(
            "Compressed video backlog",
            format!(
                "{} messages / {} ms",
                bud.video_backlog, bud.video_backlog_ms
            ),
        );
        row("Skipped encoded pictures", bud.skipped_video.to_string());
        row("Audio receive pool drops", bud.audio_pool_drops.to_string());
        row("Input queue drops", bud.input_drops.to_string());
    } else {
        webrtc_network_rows(s, &mut row);
    }
    #[cfg(not(feature = "bud"))]
    webrtc_network_rows(s, &mut row);
    row("VIDEO", String::new());
    let video = s.video.as_ref();
    row(
        "Codec",
        s.codec
            .or_else(|| video.and_then(|x| x.codec))
            .map(|c| c.name())
            .unwrap_or("Not reported")
            .into(),
    );
    row(
        "Profile",
        s.profile
            .map(|p| {
                if s.codec == Some(crate::video_stream::Codec::Hevc) {
                    match p {
                        1 => "Main".into(),
                        2 => "Main 10".into(),
                        4 => "Range extensions".into(),
                        _ => format!("HEVC profile {p}"),
                    }
                } else {
                    match p {
                        66 => "Baseline".into(),
                        77 => "Main".into(),
                        100 => "High".into(),
                        _ => format!("H.264 profile {p}"),
                    }
                }
            })
            .unwrap_or_else(|| "Not reported".into()),
    );
    row(
        "HEVC advertised",
        if s.hevc_advertised { "Yes" } else { "No" }.into(),
    );
    row(
        "Resolution",
        video
            .and_then(|x| x.width.zip(x.height))
            .map(|(w, h)| format!("{w} × {h}"))
            .unwrap_or_else(|| "Not reported".into()),
    );
    row(
        "Decoder",
        video
            .and_then(|x| x.decoder)
            .unwrap_or("Not reported")
            .into(),
    );
    if video.is_some_and(|v| v.decoder_fallback.is_some()) {
        row("Decoder fallback", "Compatibility decoder in use".into());
    }
    if let Some(depth) = video.and_then(|v| v.decoded_bit_depth) {
        row("Decoded color depth", format!("{depth}-bit"));
    }
    if let Some(chroma) = video.and_then(|v| v.decoded_chroma) {
        row("Decoded chroma", chroma.into());
    }
    if let Some(depth) = video.and_then(|v| v.display_rgb_bit_depth) {
        row(
            "RGB output",
            format!("{depth}-bit SDR (application output)"),
        );
    }
    if video.is_some_and(|v| v.display_output_fallback.is_some()) {
        row(
            "RGB output fallback",
            "Using compatible 8-bit output".into(),
        );
    }
    row(
        "Renderer",
        video
            .and_then(|x| x.renderer)
            .unwrap_or("Not reported")
            .into(),
    );
    row(
        "GPU adapter",
        video
            .and_then(|x| x.adapter.clone())
            .unwrap_or_else(|| "Not reported".into()),
    );
    if let Some(status) = video.and_then(|v| v.zero_copy_status.as_deref()) {
        row(
            "Zero copy (experimental)",
            if status.starts_with("Original NV12") {
                "Active"
            } else if status.starts_with("Starting") {
                "Starting"
            } else {
                "Unavailable; using normal GPU rendering"
            }
            .into(),
        );
    }
    if video.is_some_and(|v| v.recovering) {
        row(
            "Video status",
            "Recovering; waiting for a new keyframe.".into(),
        );
    }
    if let Some(stage) = video.and_then(|x| x.failure_stage) {
        row(
            "Video status",
            match stage {
                "video-selected-gpu-unavailable-select-automatic-in-settings" => {
                    "Selected GPU unavailable; reselect in Settings.".into()
                }
                "video-d3d11-device" | "video-decoder-not-d3d11-aware" => {
                    "Video GPU unavailable or unsupported; try another GPU.".into()
                }
                _ => "Playback stopped. Reconnect or select another GPU.".into(),
            },
        );
    }
    row(
        "Hardware decode",
        video
            .map(|x| match x.hardware_decode_status() {
                "Likely (D3D11 decoder surface)" => "Likely (GPU decoder output)",
                status => status,
            })
            .unwrap_or("Not reported")
            .into(),
    );
    row(
        "Decoder output",
        video
            .and_then(|x| x.d3d11_decoder_surface)
            .map(|x| {
                if x {
                    "D3D11 decoder surface"
                } else {
                    "DXGI surface"
                }
            })
            .unwrap_or("Not reported")
            .into(),
    );
    row(
        "Local dropped frames",
        video
            .map(|x| x.frames_dropped.to_string())
            .unwrap_or_else(|| "Not reported".into()),
    );
    if let Some(v) = video {
        if let Some(mode) = v.presentation_mode {
            row("Display timing", mode.into());
        }
        if v.local_timing.decoder_service.count > 0 {
            row("LOCAL VIDEO TIMING", String::new());
            row("CPU timing values", "Latest / average / peak (ms)".into());
            for (name, value) in [
                ("Decoder/device setup", &v.local_timing.decoder_setup),
                ("Compressed queue wait", &v.local_timing.compressed_wait),
                ("Decoder API service", &v.local_timing.decoder_service),
                ("Decoder input to output", &v.local_timing.decoder_residence),
                ("Decoded surface handoff", &v.local_timing.surface_dispatch),
                (
                    "Presentation preparation",
                    &v.local_timing.presentation_prepare,
                ),
                ("Present API call", &v.local_timing.present_call),
                (
                    "Present submission interval",
                    &v.local_timing.present_interval,
                ),
            ] {
                row(
                    name,
                    if value.count == 0 {
                        "Not reported".into()
                    } else {
                        format!(
                            "{:.2} / {:.2} / {:.2}",
                            value.latest_ms, value.average_ms, value.peak_ms
                        )
                    },
                );
            }
            row(
                "Display catch-up skips",
                v.display_updates_skipped.to_string(),
            );
            row(
                "Timing scope",
                "CPU wall time; excludes GPU completion and scanout".into(),
            );
        }
    }
    row("AUDIO", String::new());
    for (name, value) in [
        ("Codec", s.audio_codec.unwrap_or("Opus")),
        ("Sample rate", "48,000 Hz"),
    ] {
        row(
            name,
            if s.audio_ready {
                value.into()
            } else {
                "Not reported".into()
            },
        );
    }
    row(
        "Channels",
        if s.audio_ready {
            s.audio_layout.map_or_else(
                || "2 (stereo)".into(),
                |layout| match layout.channels {
                    6 => "6 (5.1)".into(),
                    8 => "8 (7.1)".into(),
                    _ => "2 (stereo)".into(),
                },
            )
        } else {
            "Not reported".into()
        },
    );
    row("Measured audio traffic", number(v.audio, "kbps"));
    if let Some(audio) = &s.audio_playback {
        if let Some(layout) = audio
            .layout
            .filter(|layout| Some(*layout) != s.audio_layout)
        {
            row("Playback channels", layout.channels.to_string());
        }
        row(
            "Playback buffer",
            number(
                Some((audio.queued_frames as f64 + audio.device_padding as f64) / 48.0),
                "ms",
            ),
        );
        row(
            "Buffer limits",
            format!(
                "{} / {} ms{}",
                audio.minimum_ms,
                audio.maximum_ms,
                if audio.custom_buffer {
                    " (custom)"
                } else {
                    " (automatic)"
                }
            ),
        );
        row("Audio underruns", audio.underruns.to_string());
        row(
            "Audio device buffer",
            number(Some(audio.device_capacity_frames as f64 / 48.0), "ms"),
        );
    }
    if let Some(ms) = s.audio_decoded_queue_ms {
        row("Decoded audio waiting", number(Some(ms), "ms"));
    }
    #[cfg(feature = "bud")]
    if let Some(bud) = &s.bud {
        if let Some(cipher) = bud.cipher {
            row("SECURITY", String::new());
            row("Media encryption", cipher.into());
        }
    } else {
        webrtc_security_rows(s, &mut row);
    }
    #[cfg(not(feature = "bud"))]
    webrtc_security_rows(s, &mut row);
    row("APP PERFORMANCE", String::new());
    row("CPU (all cores)", number(cpu.0, "%"));
    row("GPU (busiest engine)", number(cpu.1, "%"));
    #[cfg(feature = "bud")]
    if s.bud.is_some() {
        rows.retain(|(_, value)| value != "Not reported" && value != "—");
    }
    rows
}
pub fn compact_metrics(
    sample: Option<&crate::stats::Sample>,
    rates: &crate::stats::Values,
) -> String {
    let metric = |value: Option<f64>, unit: &str| {
        value
            .filter(|v| v.is_finite() && *v >= 0.0)
            .map_or_else(|| "N/A".into(), |v| format!("{v:.1}{unit}"))
    };
    let decode = sample
        .and_then(|s| s.video.as_ref())
        .map(|v| &v.local_timing.decoder_service)
        .filter(|v| v.count > 0)
        .map(|v| v.latest_ms);
    let encode = sample
        .and_then(|s| s.bud.as_ref())
        .and_then(|b| b.encode_ms);
    format!(
        "Decode CPU {} | Encode {}\nRTT {} | RX {} | FPS {}",
        metric(decode, "ms"),
        metric(encode, "ms"),
        metric(sample.and_then(|s| s.rtt_ms), "ms"),
        metric(rates.incoming, "Mbps"),
        metric(rates.fps, "")
    )
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_preserve_units_unknowns_and_stale_connection_policy() {
        let timing = crate::frame_timing::Shared::default().snapshot(0);
        let mut sample = Sample {
            state: "connected".into(),
            rtt_ms: Some(2.5),
            ..Default::default()
        };
        let values = Values {
            fps: Some(60.0),
            incoming: Some(f64::NAN),
            outgoing: Some(f64::INFINITY),
            ..Default::default()
        };
        let view = Presentation::new(&sample, &values, (None, None, None), None, &timing);
        assert_eq!(view.status, "connected");
        assert_eq!(
            view.cards,
            [
                ("Connection RTT", "2.5 ms".into()),
                ("Decoded video", "60.0 fps".into()),
                ("Incoming traffic", "—".into()),
                ("Outgoing traffic", "—".into()),
            ]
        );
        sample.at = std::time::Instant::now() - std::time::Duration::from_secs(6);
        let view = Presentation::new(&sample, &values, (None, None, None), None, &timing);
        assert_eq!(view.status, "Waiting for fresh telemetry");
        assert!(view.cards.iter().all(|(_, value)| value == "—"));
        assert!(view
            .rows
            .contains(&("Route".into(), "Unverified (telemetry unavailable)".into())));
        sample.state = "Disconnected".into();
        let view = Presentation::new(&sample, &values, (None, None, None), None, &timing);
        assert_eq!(view.status, "Disconnected");
        assert_eq!(view.cards[0].1, "2.5 ms");
    }
    #[cfg(feature = "bud")]
    #[test]
    fn bud_cards_hide_missing_values_before_blanking_stale_telemetry() {
        let timing = crate::frame_timing::Shared::default().snapshot(0);
        let mut sample = Sample {
            state: "connected".into(),
            rtt_ms: Some(10.0),
            bud: Some(evoparseq_bud::bud::Snapshot::default()),
            ..Default::default()
        };
        let values = Values {
            fps: Some(60.0),
            outgoing: Some(0.0),
            ..Default::default()
        };
        let view = Presentation::new(&sample, &values, (None, None, None), None, &timing);
        assert_eq!(
            view.cards
                .iter()
                .map(|(label, _)| *label)
                .collect::<Vec<_>>(),
            ["Connection RTT", "Decoded video", "Outgoing traffic"]
        );
        assert!(view.footer.contains("Stream framing padding is included"));
        assert!(view.footer.contains("not proven packet loss"));
        assert!(!view.rows.iter().any(|(label, _)| label == "DTLS state"));
        sample.at = std::time::Instant::now() - std::time::Duration::from_secs(6);
        let view = Presentation::new(&sample, &values, (None, None, None), None, &timing);
        assert_eq!(view.cards.len(), 3);
        assert!(view.cards.iter().all(|(_, value)| value == "—"));
    }
    #[test]
    fn memory_and_host_diagnostics_keep_order_units_and_scope_without_sampling() {
        let timing = crate::frame_timing::Shared::default();
        timing.begin(7);
        timing.set_requested(7, true);
        timing.receive(
            7,
            &crate::frame_timing::tests::packet(0, 0, &[12., 2., 4., 16.]),
        );
        let report = timing.snapshot(7);
        let sample = Sample::default();
        let memory = MemoryUsage {
            working_set_bytes: 3 * 1_048_576,
            private_committed_bytes: 5 * 1_048_576,
            private_working_set_bytes: None,
        };
        let view = Presentation::new(
            &sample,
            &Values::default(),
            (Some(2.0), Some(3.0), None),
            Some(memory),
            &report,
        );
        let position = |label: &str| view.rows.iter().position(|(key, _)| key == label).unwrap();
        assert!(position("APP PERFORMANCE") < position("Memory resident (total)"));
        assert!(position("Memory committed (private)") < position("HOST TIMING"));
        assert!(view
            .rows
            .contains(&("Memory resident (total)".into(), "3.0 MiB".into())));
        assert!(view
            .rows
            .contains(&("CPU (all cores)".into(), "2.0 %".into())));
        assert!(!view
            .rows
            .iter()
            .any(|(label, _)| label == "Memory resident (private)"));
        assert!(view
            .rows
            .contains(&("Host encode".into(), "4.00 / 4.00 / 4.00".into())));
        assert!(view.footer.contains("round-trip time, not ICMP ping"));
        assert!(view
            .footer
            .contains("not capture-to-display latency or GPU execution times"));
        assert!(!view.footer.contains("Stream framing padding"));
    }

    #[test]
    fn timing_rows_preserve_unknown_and_stale_values_and_do_not_invent_audio_encode_time() {
        let bus = crate::frame_timing::Shared::default();
        bus.begin(1);
        assert!(super::host_timing_rows(&bus.snapshot(1)).is_empty());
        bus.set_requested(1, true);
        let waiting = super::host_timing_rows(&bus.snapshot(1));
        assert!(waiting
            .iter()
            .any(|(k, v)| k == "Video timing" && v == "Not reported by host"));
        bus.receive(
            1,
            &crate::frame_timing::tests::packet(0, 0, &[12., 2., 4., 16.]),
        );
        bus.receive(
            1,
            &crate::frame_timing::tests::packet(0, 0, &[20., 3., 6., 32.]),
        );
        bus.receive(1, &crate::frame_timing::tests::packet(1, 0, &[8., 0., 20.]));
        let mut report = bus.snapshot(1);
        report.video[0].as_mut().unwrap().at =
            std::time::Instant::now() - std::time::Duration::from_secs(5);
        let rows = super::host_timing_rows(&report);
        assert!(rows
            .iter()
            .any(|(k, v)| k == "Host encode" && v == "6.00 / 5.00 / 6.00"));
        assert!(rows
            .iter()
            .any(|(k, v)| k == "Video frame interval" && v == "32.00 / 24.00 / 32.00"));
        assert!(rows
            .iter()
            .any(|(k, v)| k == "Video report age" && v.starts_with("Stale")));
        assert!(rows
            .iter()
            .any(|(k, v)| k == "Audio processing" && v == "8.00 / 8.00 / 8.00"));
        assert!(!rows.iter().any(|(k, _)| k == "Audio encode"));
        bus.failed(1);
        assert!(super::host_timing_rows(&bus.snapshot(1))
            .iter()
            .any(|(_, v)| v.contains("request failed")));
    }
    #[test]
    fn rgb_output_precision_is_observed_separately_from_decoded_precision() {
        let mut sample = Sample {
            video: Some(crate::video_output::Snapshot {
                decoded_bit_depth: Some(10),
                ..Default::default()
            }),
            ..Default::default()
        };
        let displayed = rows(&sample, &Values::default(), (None, None, None));
        assert!(!displayed.iter().any(|(k, _)| k == "RGB output"));
        let video = sample.video.as_mut().unwrap();
        video.display_rgb_bit_depth = Some(10);
        let displayed = rows(&sample, &Values::default(), (None, None, None));
        assert!(displayed
            .iter()
            .any(|(k, v)| k == "RGB output" && v.starts_with("10-bit SDR")));
        let video = sample.video.as_mut().unwrap();
        video.display_rgb_bit_depth = Some(8);
        video.display_output_fallback = Some("Driver rejected RGB10: 0x80070057".into());
        let displayed = rows(&sample, &Values::default(), (None, None, None));
        assert!(displayed
            .iter()
            .any(|(k, v)| k == "Decoded color depth" && v == "10-bit"));
        assert!(displayed
            .iter()
            .any(|(k, v)| k == "RGB output" && v.starts_with("8-bit SDR")));
        assert!(displayed.iter().any(|(k, _)| k == "RGB output fallback"));
        assert!(!displayed.iter().any(|(_, v)| v.contains("0x80070057")));
    }

    #[test]
    fn playback_status_is_readable_without_exposing_internal_error_details() {
        let mut sample = Sample {
            video: Some(crate::video_output::Snapshot {
                decoder_fallback: Some("Intel GPU renderer fell behind; internal detail".into()),
                zero_copy_status: Some(
                    "Original NV12 surfaces; no app conversion pass. Scanout not verified.".into(),
                ),
                failure_stage: Some("video-vendor-process"),
                failure_hresult: Some("0x80004005".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let displayed = rows(&sample, &Values::default(), (None, None, None));
        assert!(displayed
            .iter()
            .any(|(k, v)| k == "Zero copy (experimental)" && v == "Active"));
        assert!(displayed
            .iter()
            .any(|(k, v)| k == "Decoder fallback" && v == "Compatibility decoder in use"));
        assert!(!displayed.iter().any(|(_, v)| v.contains("internal detail")
            || v.contains("Scanout")
            || v.contains("video-vendor-process")
            || v.contains("0x80004005")));
        // Detailed telemetry is retained; only its UI presentation changes.
        assert_eq!(
            sample.video.as_ref().unwrap().failure_hresult.as_deref(),
            Some("0x80004005")
        );
        sample.video.as_mut().unwrap().zero_copy_status =
            Some("Fallback: unsupported allocation".into());
        let displayed = rows(&sample, &Values::default(), (None, None, None));
        assert!(displayed
            .iter()
            .any(|(k, v)| k == "Zero copy (experimental)"
                && v == "Unavailable; using normal GPU rendering"));
    }

    #[cfg(feature = "bud")]
    #[test]
    fn bud_statistics_use_native_counters_and_hide_ice_dtls_and_unknowns() {
        let mut native = evoparseq_bud::bud::Snapshot {
            connected: true,
            relay: true,
            relay_transport: Some("tls"),
            cipher: Some("AES-256-GCM"),
            ..Default::default()
        };
        native.channels[1].late = 7;
        native.channels[1].nacks = 2;
        let sample = Sample {
            bud: Some(native),
            dtls: "connected".into(),
            ..Default::default()
        };
        let rows = rows(&sample, &Values::default(), (None, None, None));
        assert!(rows
            .iter()
            .any(|(k, v)| k == "Video late fragments" && v == "7"));
        assert!(rows
            .iter()
            .any(|(k, v)| k == "Media encryption" && v == "AES-256-GCM"));
        assert!(rows
            .iter()
            .any(|(k, v)| k == "TURN transport" && v == "TLS"));
        assert!(!rows
            .iter()
            .any(|(k, v)| k.contains("candidate") || k.contains("DTLS") || v == "Not reported"));
        let position = rows
            .iter()
            .position(|(k, _)| k == "Host encode time")
            .unwrap();
        assert_eq!(rows[position].1, "Waiting for host");
        let mut updated = sample.clone();
        updated.bud.as_mut().unwrap().encode_ms = Some(3.4);
        let refreshed = super::rows(&updated, &Values::default(), (None, None, None));
        assert_eq!(refreshed.len(), rows.len());
        assert_eq!(
            refreshed[position],
            ("Host encode time".into(), "3.4 ms".into())
        );
    }

    #[test]
    fn silence_retains_audio_format_and_unknowns_stay_unknown() {
        let sample = Sample {
            audio_ready: true,
            route: "Direct — no TURN",
            ..Default::default()
        };
        let rows = rows(&sample, &Values::default(), (None, None, None));
        assert!(rows.contains(&("Codec".into(), "Opus".into())));
        assert!(rows.contains(&("TURN server in use".into(), "None".into())));
        assert!(rows.iter().all(|(label, _)| ![
            "Configured bitrate",
            "Packets lost",
            "DTLS version / cipher",
            "GPU video decode"
        ]
        .contains(&label.as_str())));
    }
    #[test]
    fn compact_metrics_use_existing_values_and_keep_unknowns_unknown() {
        let mut sample = crate::stats::Sample {
            rtt_ms: Some(2.5),
            video: Some(Default::default()),
            ..Default::default()
        };
        let rates = crate::stats::Values {
            incoming: Some(10.0),
            fps: Some(60.0),
            ..Default::default()
        };
        let empty = compact_metrics(None, &Default::default());
        assert!(empty.contains("Decode CPU N/A"));
        sample
            .video
            .as_mut()
            .unwrap()
            .local_timing
            .decoder_service
            .record(std::time::Duration::from_millis(1));
        let text = compact_metrics(Some(&sample), &rates);
        assert!(text.contains("Decode CPU 1.0ms"));
        assert!(text.contains("Encode N/A"));
        assert!(text.contains("RTT 2.5ms"));
        assert!(text.contains("RX 10.0Mbps"));
    }
}
