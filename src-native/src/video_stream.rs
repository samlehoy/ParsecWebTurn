//! Bounded inspection of pinned Parsec video framing, not a decoder.
//! Proprietary metadata is separate from H.264/HEVC Annex B byte-stream syntax.
use crate::backend::VideoProtocol;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum Codec {
    #[default]
    H264,
    Hevc,
}
impl Codec {
    pub fn name(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::Hevc => "H.265 / HEVC",
        }
    }
}

#[derive(Clone, Default, Serialize)]
pub struct Snapshot {
    pub codec: Option<Codec>,
    pub hevc_advertised: bool,
    pub vps_observed: bool,
    pub metadata_messages: u64,
    pub encoded_messages: u64,
    pub key_chunks_announced: u64,
    pub delta_chunks_announced: u64,
    pub annex_b_messages: u64,
    pub unrecognized_messages: u64,
    pub idr_messages: u64,
    pub sps_observed: bool,
    pub pps_observed: bool,
    pub sps_profile_idc: Option<u8>,
    pub sps_constraint_flags: Option<u8>,
    pub sps_level_idc: Option<u8>,
    pub parameter_sets_and_idr_observed: bool,
    pub protocol_changes: u64,
}

#[derive(Default)]
pub struct Inspector {
    protocol: Option<VideoProtocol>,
    headers_seen: bool,
    next_key: bool,
    snapshot: Snapshot,
}

impl Inspector {
    pub fn enable_hevc(&mut self, enabled: bool) {
        self.snapshot.hevc_advertised = enabled;
    }
    pub fn configure(&mut self, protocol: VideoProtocol) {
        if self.protocol.as_ref() == Some(&protocol) {
            return;
        }
        let changes = self
            .snapshot
            .protocol_changes
            .saturating_add(u64::from(self.protocol.is_some()));
        *self = Self {
            protocol: Some(protocol),
            snapshot: Snapshot {
                protocol_changes: changes,
                hevc_advertised: self.snapshot.hevc_advertised,
                ..Default::default()
            },
            ..Default::default()
        };
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.clone()
    }

    pub fn receive(&mut self, bytes: &[u8]) -> Option<PacketInfo> {
        if let Some(protocol) = &self.protocol {
            let word = |offset: u32| -> Option<u32> {
                let start = usize::try_from(offset).ok()?;
                Some(u32::from_le_bytes(
                    bytes.get(start..start.checked_add(4)?)?.try_into().ok()?,
                ))
            };
            if bytes.len() == protocol.message_size as usize
                && word(protocol.version_offset) == Some(protocol.version)
            {
                if let Some(flags) = word(protocol.flag_offset) {
                    self.headers_seen = true;
                    self.next_key = flags & 2 != 0;
                    self.snapshot.metadata_messages =
                        self.snapshot.metadata_messages.saturating_add(1);
                    return None;
                }
            }
        }
        self.snapshot.encoded_messages = self.snapshot.encoded_messages.saturating_add(1);
        let announced_key = !self.headers_seen || self.next_key;
        self.next_key = false;
        if announced_key {
            self.snapshot.key_chunks_announced =
                self.snapshot.key_chunks_announced.saturating_add(1);
        } else {
            self.snapshot.delta_chunks_announced =
                self.snapshot.delta_chunks_announced.saturating_add(1);
        }
        // HEVC VPS (type 32) cannot be a valid H.264 NAL (type 0).
        // Only recognize HEVC after advertising it, and then lock the codec.
        let hevc = self.snapshot.hevc_advertised
            && (self.snapshot.codec == Some(Codec::Hevc)
                || (self.snapshot.codec.is_none() && hevc_annex_b(bytes).is_some_and(|s| s.vps)));
        let Some(summary) = (if hevc {
            hevc_annex_b(bytes)
        } else {
            annex_b(bytes)
        }) else {
            self.snapshot.unrecognized_messages =
                self.snapshot.unrecognized_messages.saturating_add(1);
            return None;
        };
        self.snapshot.annex_b_messages = self.snapshot.annex_b_messages.saturating_add(1);
        let codec = if hevc { Codec::Hevc } else { Codec::H264 };
        if summary.sps.is_some() || summary.vps {
            self.snapshot.codec = Some(codec);
        }
        self.snapshot.vps_observed |= summary.vps;
        if let Some([profile, constraints, level]) = summary.sps {
            self.snapshot.sps_observed = true;
            self.snapshot.sps_profile_idc = Some(profile);
            if codec == Codec::H264 {
                self.snapshot.sps_constraint_flags = Some(constraints);
                self.snapshot.sps_level_idc = Some(level);
            }
        }
        self.snapshot.pps_observed |= summary.pps;
        if summary.idr {
            self.snapshot.idr_messages = self.snapshot.idr_messages.saturating_add(1);
        }
        self.snapshot.parameter_sets_and_idr_observed = self.snapshot.sps_observed
            && self.snapshot.pps_observed
            && self.snapshot.idr_messages != 0
            && (codec != Codec::Hevc || self.snapshot.vps_observed);
        Some(PacketInfo {
            codec,
            idr: summary.idr,
            parameters: summary.vps || summary.sps.is_some() || summary.pps,
        })
    }
}

#[derive(Clone, Copy)]
pub struct PacketInfo {
    pub codec: Codec,
    pub idr: bool,
    pub parameters: bool,
}

#[derive(Default)]
struct NalSummary {
    vps: bool,
    sps: Option<[u8; 3]>,
    pps: bool,
    idr: bool,
}

// ITU-T H.265 Annex B / NAL-unit header. Single-layer Main-profile path.
// Parameter sets and IRAP are read from syntax, never from the requested codec.
fn hevc_annex_b(bytes: &[u8]) -> Option<NalSummary> {
    let mut summary = NalSummary::default();
    let mut offset = 0;
    for _ in 0..256 {
        while bytes.get(offset) == Some(&0) {
            offset += 1;
        }
        if offset < 2 || bytes.get(offset) != Some(&1) {
            return None;
        }
        let begin = offset + 1;
        let mut end = begin;
        while end < bytes.len()
            && !bytes[end..].starts_with(&[0, 0, 1])
            && !bytes[end..].starts_with(&[0, 0, 0, 1])
        {
            end += 1;
        }
        let nal = bytes.get(begin..end)?;
        let header = *nal.first()?;
        let second = *nal.get(1)?;
        if header & 0x81 != 0 || second & 0xf8 != 0 || second & 7 == 0 || nal.len() < 3 {
            return None;
        }
        match (header >> 1) & 63 {
            32 => summary.vps = true,
            33 => {
                // profile_tier_level begins after the first SPS RBSP byte.
                // Main/Main10 profile IDs are distinct from AVC profile IDs.
                let profile = *nal.get(3)? & 31;
                summary.sps = Some([profile, 0, 0]);
            }
            34 => summary.pps = true,
            16..=21 => summary.idr = true, // BLA, IDR or CRA random access.
            0..=31 | 35..=40 => {}
            _ => return None,
        }
        if end == bytes.len() {
            return Some(summary);
        }
        offset = end;
    }
    None
}

// No allocation or payload retention. A bounded linear scan accepts 3/4-byte
// start codes and leading/trailing zero bytes; unknown syntax stays unknown.
fn annex_b(bytes: &[u8]) -> Option<NalSummary> {
    fn start(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
        let mut i = from;
        while i.checked_add(3)? <= bytes.len() {
            if bytes[i..].starts_with(&[0, 0, 0, 1]) {
                return Some((i, 4));
            }
            if bytes[i..].starts_with(&[0, 0, 1]) {
                return Some((i, 3));
            }
            i += 1;
        }
        None
    }
    let (mut offset, mut prefix) = start(bytes, 0)?;
    if bytes[..offset].iter().any(|b| *b != 0) {
        return None;
    }
    let mut summary = NalSummary::default();
    for _ in 0..256 {
        let begin = offset + prefix;
        let next = start(bytes, begin);
        let end = next.map_or(bytes.len(), |(i, _)| i);
        let nal = bytes.get(begin..end)?;
        let header = *nal.first()?;
        let kind = header & 31;
        if header & 128 != 0 || !(1..=23).contains(&kind) {
            return None;
        }
        match kind {
            7 => summary.sps = Some(nal.get(1..4)?.try_into().ok()?),
            8 => {
                if nal.len() < 2 {
                    return None;
                }
                summary.pps = true;
            }
            5 => {
                if nal.len() < 2 {
                    return None;
                }
                summary.idr = true;
            }
            _ => {}
        }
        match next {
            Some((i, p)) => {
                offset = i;
                prefix = p;
            }
            None => return Some(summary),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    const HEVC_KEY: &[u8] = &[
        0, 0, 0, 1, 0x40, 1, 0x80, // VPS
        0, 0, 1, 0x42, 1, 1, 1, 0x80, // SPS, Main profile
        0, 0, 1, 0x44, 1, 0x80, // PPS
        0, 0, 1, 0x26, 1, 0x80, // IDR_W_RADL
    ];
    #[test]
    fn hevc_requires_advertisement_and_observed_parameter_sets() {
        let mut inspector = Inspector::default();
        assert!(inspector.receive(HEVC_KEY).is_none());
        assert_eq!(inspector.snapshot().codec, None);
        inspector.enable_hevc(true);
        inspector.configure(protocol());
        assert!(inspector.snapshot().hevc_advertised);
        let mut with_aud = Inspector::default();
        with_aud.enable_hevc(true);
        let mut aud = vec![0, 0, 0, 1, 0x46, 1, 0x50];
        aud.extend_from_slice(HEVC_KEY);
        assert_eq!(with_aud.receive(&aud).unwrap().codec, Codec::Hevc);
        let info = inspector.receive(&HEVC_KEY[..7]).unwrap();
        assert_eq!(info.codec, Codec::Hevc);
        assert!(info.parameters); // Separate VPS must reach the decoder before IDR.
        assert!(!info.idr);
        let info = inspector.receive(&HEVC_KEY[7..]).unwrap();
        assert!(info.idr && info.parameters);
        let snapshot = inspector.snapshot();
        assert_eq!(snapshot.codec, Some(Codec::Hevc));
        assert_eq!(snapshot.sps_profile_idc, Some(1));
        assert!(snapshot.vps_observed && snapshot.parameter_sets_and_idr_observed);
        assert!(inspector.receive(&[0, 0, 1, 0x02, 1, 0x80]).is_some());
        assert!(inspector.receive(&[0, 0, 1, 0x02, 0, 0x80]).is_none());
        assert!(inspector.receive(KEY).is_none()); // No ambiguous codec change.
    }
    #[test]
    fn advertising_hevc_still_accepts_host_h264_fallback() {
        let mut inspector = Inspector::default();
        inspector.enable_hevc(true);
        assert_eq!(inspector.receive(KEY).unwrap().codec, Codec::H264);
        assert_eq!(inspector.snapshot().codec, Some(Codec::H264));
        assert!(inspector.receive(HEVC_KEY).is_none());
        for bytes in [
            &[0, 0, 1, 0x40][..],
            &[0, 0, 1, 0x40, 0, 0x80],
            &[0, 0, 1, 0x40, 9, 0x80],
        ] {
            let mut unknown = Inspector::default();
            unknown.enable_hevc(true);
            assert!(unknown.receive(bytes).is_none());
            assert_eq!(unknown.snapshot().codec, None);
        }
    }
    fn protocol() -> VideoProtocol {
        VideoProtocol {
            version: 0,
            message_size: 11,
            version_offset: 0,
            flag_offset: 4,
        }
    }
    fn metadata(key: bool) -> Vec<u8> {
        let mut v = vec![0; 11];
        v[4] = if key { 2 } else { 0 };
        v
    }
    // Header-only synthetic fixture, intentionally not a decodable picture.
    const KEY: &[u8] = &[
        0, 0, 0, 1, 0x67, 100, 0, 40, 0x80, 0, 0, 1, 0x68, 0x80, 0, 0, 0, 1, 0x65, 0x80,
    ];
    #[test]
    fn pinned_headers_do_not_enter_encoded_stream_and_key_flag_is_consumed() {
        let mut inspector = Inspector::default();
        inspector.configure(protocol());
        let _ = inspector.receive(&metadata(true));
        let _ = inspector.receive(KEY);
        let _ = inspector.receive(&[0, 0, 1, 0x41, 0x80]);
        let s = inspector.snapshot();
        assert_eq!(
            (
                s.metadata_messages,
                s.encoded_messages,
                s.key_chunks_announced,
                s.delta_chunks_announced
            ),
            (1, 2, 1, 1)
        );
        assert_eq!((s.annex_b_messages, s.idr_messages), (2, 1));
        assert!(s.parameter_sets_and_idr_observed);
        assert_eq!(s.sps_profile_idc, Some(100));
        assert_eq!(s.sps_level_idc, Some(40));
        assert!(!serde_json::to_string(&s).unwrap().contains("payload"));
    }
    #[test]
    fn headerless_key_fallback_matches_pinned_client_without_claiming_idr() {
        let mut inspector = Inspector::default();
        inspector.configure(protocol());
        let _ = inspector.receive(&[0, 0, 1, 0x41, 0x80]);
        let _ = inspector.receive(&[0, 0, 1, 0x41, 0x80]);
        let s = inspector.snapshot();
        assert_eq!(s.key_chunks_announced, 2);
        assert_eq!(s.idr_messages, 0);
        assert!(!s.parameter_sets_and_idr_observed);
    }
    #[test]
    fn malformed_unknown_and_excessive_nals_are_nonfatal_and_not_trusted() {
        for data in [
            vec![],
            vec![0, 0, 1],
            vec![0, 0, 1, 0x80],
            vec![0, 0, 1, 0x67],
            vec![0, 0, 1, 0x65],
            b"opaque data".to_vec(),
            [0, 0, 1, 0x41, 0x80].repeat(257),
        ] {
            let mut inspector = Inspector::default();
            let _ = inspector.receive(&data);
            let s = inspector.snapshot();
            assert_eq!(s.unrecognized_messages, 1);
            assert_eq!(s.annex_b_messages, 0);
            assert!(!s.parameter_sets_and_idr_observed);
        }
    }
    #[test]
    fn repeated_configuration_preserves_state_and_changed_protocol_resets_it() {
        let mut inspector = Inspector::default();
        inspector.configure(protocol());
        let _ = inspector.receive(KEY);
        inspector.configure(protocol());
        assert_eq!(inspector.snapshot().idr_messages, 1);
        let mut p = protocol();
        p.version = 1;
        inspector.configure(p);
        assert_eq!(inspector.snapshot().protocol_changes, 1);
        assert_eq!(inspector.snapshot().idr_messages, 0);
    }
    #[test]
    fn invalid_offsets_and_wrong_version_never_panic_or_match_metadata() {
        let mut inspector = Inspector::default();
        let mut p = protocol();
        p.flag_offset = u32::MAX;
        inspector.configure(p);
        let _ = inspector.receive(&metadata(true));
        assert_eq!(inspector.snapshot().metadata_messages, 0);
        inspector.configure(protocol());
        let mut v = metadata(true);
        v[0] = 3;
        let _ = inspector.receive(&v);
        assert_eq!(inspector.snapshot().metadata_messages, 0);
    }
}
