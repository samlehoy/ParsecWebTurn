//! Cheap control-plane formatting; no frame work or GPU queries.
use serde_json::{json, Value};
pub const AUTO_HIDE_MS: u64 = 5000;
pub fn metrics(
    fps: Option<f64>,
    rtt: Option<f64>,
    mbps: f64,
    codec: &str,
    width: u32,
    height: u32,
    depth: u32,
) -> Value {
    let value = |n: Option<f64>, unit: &str| {
        n.filter(|v| v.is_finite() && *v >= 0.)
            .map(|v| format!("{v:.1} {unit}"))
            .unwrap_or_else(|| format!("N/A {unit}"))
    };
    json!({"fps":value(fps,"FPS"),"rtt":value(rtt,"ms"),"bitrate":value(Some(mbps),"Mbit/s"),"format":format!("{codec} | {width} x {height} | {depth}-bit"),"auto_hide_ms":AUTO_HIDE_MS})
}
/// A configured host rate is a Surface hint, not measured FPS. Dynamic/unknown
/// input must not be advertised as a fixed-cadence source to the compositor.
pub fn frame_rate_hint(host: &Value) -> f64 {
    if host["constant_fps"] != true {
        return 0.;
    }
    host["fps"]
        .as_u64()
        .filter(|n| (1..=1000).contains(n))
        .map(|n| n as f64)
        .unwrap_or(0.)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn surface_rate_uses_host_configuration_not_requested_or_measured_fps() {
        assert_eq!(frame_rate_hint(&json!({"fps":60,"constant_fps":true})), 60.);
        assert_eq!(
            frame_rate_hint(&json!({"fps":120,"constant_fps":true})),
            120.
        );
        for host in [
            json!({"fps":60,"constant_fps":false}),
            json!({"fps":60}),
            json!({"constant_fps":true}),
            json!({"fps":0,"constant_fps":true}),
            json!({"fps":-1,"constant_fps":true}),
            json!({"fps":1001,"constant_fps":true}),
            Value::Null,
        ] {
            assert_eq!(frame_rate_hint(&host), 0.);
        }
    }
    #[test]
    fn missing_and_invalid_metrics_remain_unknown() {
        let m = metrics(None, Some(f64::NAN), 12.5, "HEVC", 1920, 1080, 10);
        assert_eq!(m["rtt"], "N/A ms");
        assert_eq!(m["bitrate"], "12.5 Mbit/s");
    }
}

#[cfg(target_os = "android")]
pub static REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
