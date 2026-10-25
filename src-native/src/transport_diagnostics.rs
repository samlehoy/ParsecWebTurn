//! Optional, version-pinned library diagnostics. Never used to control a peer.
//! Only fixed categories survive; raw messages are neither printed nor retained.
use std::{
    fmt::Write,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

const TARGET: &str = "webrtc::peer_connection::peer_connection_internal";
const CRYPTO_TARGET: &str = "dtls::crypto";
const LIMIT: usize = 16;
static ENABLED: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
const ICE_TARGET: &str = "webrtc_ice::agent::agent_internal";
#[cfg(test)]
const SELECTOR_TARGET: &str = "webrtc_ice::agent::agent_selector";
#[cfg(test)]
const TURN_TARGET: &str = "turn::client::relay_conn";
#[cfg(test)]
const CHECK_LABELS: [&str; 15] = [
    "binding_check_attempt_events",
    "stun_send_failure_events",
    "permission_403_events",
    "permission_other_error_events",
    "inbound_binding_request_events",
    "authenticated_success_events",
    "rejected_binding_events",
    "role_conflict_events",
    "transaction_source_mismatch_events",
    "permission_transaction_events",
    "channel_bind_success_events",
    "channel_bind_failure_events",
    "missing_candidate_pair_events",
    "expired_binding_batch_events",
    "oversized_log_events",
];

#[cfg(test)]
fn check_categories(target: &str, message: &str) -> Vec<usize> {
    let body = message.split_once("]: ").map_or(message, |(_, body)| body);
    if target == ICE_TARGET {
        if body.starts_with("ping STUN from ") {
            return vec![0];
        }
        if let Some(error) = body.strip_prefix("failed to send STUN message: ") {
            let mut categories = vec![1];
            if error.starts_with("CreatePermission error response (error ") {
                categories.push(
                    if error.starts_with("CreatePermission error response (error 403:") {
                        2
                    } else {
                        3
                    },
                );
            }
            return categories;
        }
        if body.starts_with("inbound STUN (Request) from ") {
            return vec![4];
        }
        if body.starts_with("discard message from (")
            || body.starts_with("discard success message from (")
        {
            return vec![6];
        }
        if body.starts_with("inbound isControlling &&")
            || body.starts_with("inbound isControlled &&")
            || body.starts_with("useCandidate &&")
        {
            return vec![7];
        }
        if body.starts_with("pingAllCandidates called with no candidate pairs.") {
            return vec![12];
        }
        if body.starts_with("Discarded ")
            && body.ends_with(" binding requests because they expired")
        {
            return vec![13];
        }
    } else if target == SELECTOR_TARGET {
        if body.starts_with("inbound STUN (SuccessResponse) from ") {
            return vec![5];
        }
        if body.starts_with("discard message: transaction source and destination does not match") {
            return vec![8];
        }
    } else if target == TURN_TARGET {
        if body == "UDPConn.createPermissions call PerformTransaction 1" {
            return vec![9];
        }
        if body.starts_with("channel binding successful: ") {
            return vec![10];
        }
        if body.starts_with("bind() failed: ") || body.starts_with("bind() for refresh failed: ") {
            return vec![11];
        }
    }
    Vec::new()
}

#[cfg(test)]
fn check_delta(
    before: &[u64; CHECK_LABELS.len()],
    after: &[u64; CHECK_LABELS.len()],
) -> serde_json::Value {
    CHECK_LABELS
        .iter()
        .enumerate()
        .map(|(i, label)| {
            (
                (*label).to_owned(),
                serde_json::json!(after[i].saturating_sub(before[i])),
            )
        })
        .collect::<serde_json::Map<_, _>>()
        .into()
}

#[cfg(test)]
pub fn ice_pair_summary(report: &webrtc::stats::StatsReport) -> serde_json::Value {
    use webrtc::{ice::candidate::CandidatePairState as S, stats::StatsReportType};
    let mut states = [0u64; 5];
    let mut nominated = 0u64;
    let mut responses = 0u64;
    for value in report.reports.values() {
        if let StatsReportType::CandidatePair(pair) = value {
            let index = match pair.state {
                S::Unspecified => 0,
                S::Waiting => 1,
                S::InProgress => 2,
                S::Failed => 3,
                S::Succeeded => 4,
            };
            states[index] = states[index].saturating_add(1);
            nominated = nominated.saturating_add(u64::from(pair.nominated));
            responses = responses.saturating_add(pair.responses_received);
        }
    }
    // Upstream leaves requests_sent/received, packet and byte counters at
    // their defaults. Never present those placeholder zeroes as measurements.
    serde_json::json!({"source":"webrtc-public-stats", "pairs_by_state":{"unspecified":states[0],"waiting":states[1],"in_progress":states[2],"failed":states[3],"succeeded":states[4]},"nominated_pairs":nominated,"authenticated_responses":responses,"requests_sent":null,"requests_received":null,"packets_sent":null,"packets_received":null})
}

#[derive(Clone, serde::Serialize)]
pub struct Event {
    stage: &'static str,
    reason: &'static str,
}

struct Collector {
    events: Mutex<(Vec<Event>, usize)>,
    #[cfg(test)]
    checks: Mutex<[u64; CHECK_LABELS.len()]>,
}
static COLLECTOR: Collector = Collector {
    events: Mutex::new((Vec::new(), 0)),
    #[cfg(test)]
    checks: Mutex::new([0; CHECK_LABELS.len()]),
};

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Acquire)
}

pub fn contains_reason(reason: &str) -> bool {
    COLLECTOR
        .events
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .0
        .iter()
        .any(|event| event.reason == reason)
}

// Bounded transient formatting, including for unknown/dynamic library errors.
struct Buffer {
    bytes: [u8; 512],
    len: usize,
}
impl std::fmt::Write for Buffer {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        if text.len() > self.bytes.len() - self.len {
            return Err(std::fmt::Error);
        }
        self.bytes[self.len..self.len + text.len()].copy_from_slice(text.as_bytes());
        self.len += text.len();
        Ok(())
    }
}
impl Drop for Buffer {
    fn drop(&mut self) {
        self.bytes.fill(0);
    }
}

fn classify(target: &str, message: &str) -> Option<Event> {
    if target == CRYPTO_TARGET {
        return crate::transport_diagnostic_errors::signature_reason(message).map(|reason| Event {
            stage: "dtls-signature-verification",
            reason,
        });
    }
    if target != TARGET {
        return None;
    }
    let (stage, error) = [
        ("ice", "Failed to start manager ice: "),
        ("dtls", "Failed to start manager dtls: "),
        ("sctp", "Failed to start SCTP: "),
        ("data-channel", "failed to open data channel: "),
    ]
    .into_iter()
    .find_map(|(stage, prefix)| message.strip_prefix(prefix).map(|e| (stage, e)))?;
    // Compare only exact fixed Display values from the pinned public error enums.
    // Unknown/changed messages remain unknown, never a protocol decision.
    use webrtc::dtls::Error as D;
    use webrtc::Error as W;
    let known = [
        (
            W::ErrNoSRTPProtectionProfile.to_string(),
            "srtp-profile-missing",
        ),
        (
            W::ErrNoRemoteCertificate.to_string(),
            "remote-certificate-missing",
        ),
        (
            W::ErrNoMatchingCertificateFingerprint.to_string(),
            "certificate-fingerprint-mismatch",
        ),
        (
            W::ErrUnsupportedFingerprintAlgorithm.to_string(),
            "fingerprint-algorithm-unsupported",
        ),
        (
            D::ErrCipherSuiteNoIntersection.to_string(),
            "cipher-suite-no-intersection",
        ),
        (
            D::ErrInvalidSignatureAlgorithm.to_string(),
            "signature-algorithm-invalid",
        ),
        (D::ErrKeySignatureMismatch.to_string(), "signature-mismatch"),
        (
            D::ErrClientCertificateRequired.to_string(),
            "client-certificate-required",
        ),
        (
            D::ErrClientCertificateNotVerified.to_string(),
            "client-certificate-not-verified",
        ),
        (
            D::ErrServerNoMatchingSrtpProfile.to_string(),
            "srtp-profile-no-intersection",
        ),
        (
            D::ErrRequestedButNoSrtpExtension.to_string(),
            "srtp-extension-missing",
        ),
        (D::ErrAlertFatalOrClose.to_string(), "peer-alert-or-close"),
        (
            D::ErrUnsupportedProtocolVersion.to_string(),
            "protocol-version-unsupported",
        ),
        (D::ErrDeadlineExceeded.to_string(), "deadline-exceeded"),
        (D::ErrConnClosed.to_string(), "connection-closed"),
        (W::ErrSCTPTransportDTLS.to_string(), "dtls-not-established"),
    ];
    Some(Event {
        stage,
        reason: known
            .into_iter()
            .find(|(text, _)| text == error)
            .map(|(_, reason)| reason)
            .or_else(|| crate::transport_diagnostic_errors::dtls_reason(error))
            .unwrap_or("unknown"),
    })
}

impl log::Log for Collector {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        #[cfg(test)]
        if [ICE_TARGET, SELECTOR_TARGET, TURN_TARGET].contains(&metadata.target()) {
            return true;
        }
        (metadata.target() == TARGET && metadata.level() <= log::Level::Warn)
            || (metadata.target() == CRYPTO_TARGET && metadata.level() == log::Level::Trace)
    }
    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let mut buffer = Buffer {
            bytes: [0; 512],
            len: 0,
        };
        if write!(&mut buffer, "{}", record.args()).is_err() {
            #[cfg(test)]
            {
                let mut checks = self.checks.lock().unwrap_or_else(|e| e.into_inner());
                checks[14] = checks[14].saturating_add(1);
            }
            return;
        }
        let Ok(message) = std::str::from_utf8(&buffer.bytes[..buffer.len]) else {
            return;
        };
        #[cfg(test)]
        {
            let categories = check_categories(record.target(), message);
            if !categories.is_empty() {
                let mut checks = self.checks.lock().unwrap_or_else(|e| e.into_inner());
                for i in categories {
                    checks[i] = checks[i].saturating_add(1);
                }
            }
        }
        if let Some(event) = classify(record.target(), message) {
            let mut events = self.events.lock().unwrap_or_else(|e| e.into_inner());
            if events.0.len() < LIMIT {
                events.0.push(event);
            } else {
                events.1 = events.1.saturating_add(1);
            }
        }
    }
    fn flush(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ice_observations_count_fixed_categories_without_retaining_messages() {
        use log::Log;
        let collector = Collector {
            events: Mutex::new((Vec::new(), 0)),
            checks: Mutex::new([0; CHECK_LABELS.len()]),
        };
        let before = [0; CHECK_LABELS.len()];
        for _ in 0..32 {
            collector.log(
                &log::Record::builder()
                    .level(log::Level::Trace)
                    .target(ICE_TARGET)
                    .args(format_args!(
                        "[private-agent]: ping STUN from private-local to private-remote"
                    ))
                    .build(),
            );
        }
        collector.log(&log::Record::builder().level(log::Level::Trace).target(ICE_TARGET)
            .args(format_args!("[private-agent]: failed to send STUN message: CreatePermission error response (error 403: private-server-reason)")).build());
        collector.log(
            &log::Record::builder()
                .level(log::Level::Trace)
                .target(SELECTOR_TARGET)
                .args(format_args!(
                    "inbound STUN (SuccessResponse) from private-remote to private-local"
                ))
                .build(),
        );
        collector.log(
            &log::Record::builder()
                .level(log::Level::Trace)
                .target("unrelated")
                .args(format_args!(
                    "[private-agent]: ping STUN from private-local to private-remote"
                ))
                .build(),
        );
        let oversized = "private-data".repeat(100);
        collector.log(
            &log::Record::builder()
                .level(log::Level::Trace)
                .target(ICE_TARGET)
                .args(format_args!(
                    "[private-agent]: failed to send STUN message: {oversized}"
                ))
                .build(),
        );
        let after = collector.checks.lock().unwrap();
        let delta = check_delta(&before, &after);
        assert_eq!(delta["binding_check_attempt_events"], 32);
        assert_eq!(delta["stun_send_failure_events"], 1);
        assert_eq!(delta["permission_403_events"], 1);
        assert_eq!(delta["authenticated_success_events"], 1);
        assert_eq!(delta["oversized_log_events"], 1);
        assert!(!delta.to_string().contains("private"));
        assert!(collector.events.lock().unwrap().0.is_empty());
        assert_eq!(
            check_delta(&after, &after)["binding_check_attempt_events"],
            0
        );
    }

    #[test]
    fn pair_summary_excludes_identifiers_and_unimplemented_counters() {
        use webrtc::{
            ice::{agent::agent_stats::CandidatePairStats, candidate::CandidatePairState as S},
            stats::{ICECandidatePairStats, StatsReport, StatsReportType},
        };
        let mut report = StatsReport {
            reports: Default::default(),
        };
        for (i, state) in [S::InProgress, S::Failed, S::Succeeded]
            .into_iter()
            .enumerate()
        {
            let pair = CandidatePairStats {
                local_candidate_id: format!("private-local-{i}"),
                remote_candidate_id: format!("private-remote-{i}"),
                state,
                nominated: state == S::Succeeded,
                responses_received: if state == S::Succeeded { 7 } else { 0 },
                requests_sent: 999,
                packets_sent: 999,
                ..Default::default()
            };
            report.reports.insert(
                format!("private-id-{i}"),
                StatsReportType::CandidatePair(ICECandidatePairStats::from(pair)),
            );
        }
        let summary = ice_pair_summary(&report);
        assert_eq!(summary["pairs_by_state"]["in_progress"], 1);
        assert_eq!(summary["pairs_by_state"]["failed"], 1);
        assert_eq!(summary["pairs_by_state"]["succeeded"], 1);
        assert_eq!(summary["nominated_pairs"], 1);
        assert_eq!(summary["authenticated_responses"], 7);
        assert!(summary["requests_sent"].is_null());
        assert!(summary["packets_sent"].is_null());
        assert!(!summary.to_string().contains("private"));
    }

    #[test]
    fn only_exact_library_errors_become_fixed_categories() {
        let message = format!(
            "Failed to start manager dtls: {}",
            webrtc::Error::ErrNoSRTPProtectionProfile
        );
        assert_eq!(
            classify(TARGET, &message).unwrap().reason,
            "srtp-profile-missing"
        );
        assert!(classify("other-library", &message).is_none());
        assert!(classify(TARGET, "unrelated secret").is_none());
        let event = classify(TARGET, "Failed to start manager dtls: secret.example/token").unwrap();
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"stage":"dtls","reason":"unknown"}"#
        );
    }
    #[test]
    fn log_collection_is_bounded_and_oversized_errors_are_discarded() {
        let collector = Collector {
            events: Mutex::new((Vec::new(), 0)),
            checks: Mutex::new([0; CHECK_LABELS.len()]),
        };
        use log::Log;
        for _ in 0..20 {
            collector.log(
                &log::Record::builder()
                    .level(log::Level::Warn)
                    .target(TARGET)
                    .args(format_args!(
                        "Failed to start manager dtls: unrecognized-private-error"
                    ))
                    .build(),
            );
        }
        let oversized = "sensitive".repeat(100);
        collector.log(
            &log::Record::builder()
                .level(log::Level::Warn)
                .target(TARGET)
                .args(format_args!("Failed to start manager dtls: {oversized}"))
                .build(),
        );
        let events = collector.events.lock().unwrap();
        assert_eq!(events.0.len(), LIMIT);
        assert_eq!(events.1, 4);
        assert!(!serde_json::to_string(&events.0)
            .unwrap()
            .contains("private"));
    }
}
