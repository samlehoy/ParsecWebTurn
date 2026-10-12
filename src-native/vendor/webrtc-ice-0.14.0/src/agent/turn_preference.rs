//! Local TURN transport policy; relay allocation still uses standard UDP ICE.
use crate::url::{ProtoType, SchemeType, Url};
use std::time::Duration;

pub(super) fn tier(url: &Url) -> Option<usize> {
    match (url.scheme, url.proto) {
        (SchemeType::Turn, ProtoType::Udp) => Some(0),
        (SchemeType::Turns, ProtoType::Tcp) => Some(1),
        (SchemeType::Turn, ProtoType::Tcp) => Some(2),
        _ => None,
    }
}

pub(super) fn delay(url: &Url, urls: &[Url]) -> Duration {
    let Some(current) = tier(url) else {
        return Duration::ZERO;
    };
    let preceding = (0..current)
        .filter(|t| urls.iter().any(|u| tier(u) == Some(*t)))
        .count();
    Duration::from_secs(preceding as u64 * 3)
}

pub(super) fn priority(url: &Url, index: usize) -> u32 {
    // RFC 8445 section 5.1.2.1: relay type preference remains zero; only
    // local preference changes. Different configured URLs get unique values.
    let local = 65535 - tier(url).unwrap_or(2) as u32 * 16384 - index.min(16383) as u32;
    (local << 8) + 255 // component 1
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preference_is_transport_based_and_missing_tiers_do_not_delay() {
        let urls: Vec<_> = [
            "turn:example.invalid:3478?transport=tcp",
            "turns:example.invalid:443?transport=tcp",
            "turn:example.invalid:3478?transport=udp",
        ]
        .iter()
        .map(|s| Url::parse_url(s).unwrap())
        .collect();
        assert_eq!(delay(&urls[2], &urls), Duration::ZERO);
        assert_eq!(delay(&urls[1], &urls), Duration::from_secs(3));
        assert_eq!(delay(&urls[0], &urls), Duration::from_secs(6));
        assert!(priority(&urls[2], 2) > priority(&urls[1], 1));
        assert!(priority(&urls[1], 1) > priority(&urls[0], 0));
        assert_eq!(delay(&urls[0], &urls[..1]), Duration::ZERO);
        assert_eq!(delay(&urls[1], &urls[..2]), Duration::ZERO);
        assert_eq!(delay(&urls[0], &urls[..2]), Duration::from_secs(3));
    }
}
