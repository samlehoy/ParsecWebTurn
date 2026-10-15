//! Process AND adapter scoped decode engine activity. Presentation/3D activity
//! does not satisfy this gate. Missing counters always remain unknown.
use serde_json::{Value, json};
use windows::{Win32::System::Performance::*, core::w};

pub struct DecodeCounter {
    query: PDH_HQUERY,
    counter: PDH_HCOUNTER,
    prefix: String,
    valid: usize,
    positive: usize,
    maximum: f64,
}
impl DecodeCounter {
    pub fn open(luid: u64) -> Option<Self> {
        unsafe {
            let mut query = PDH_HQUERY::default();
            if PdhOpenQueryW(None, 0, &mut query) != 0 {
                return None;
            }
            let mut counter = PDH_HCOUNTER::default();
            if PdhAddEnglishCounterW(
                query,
                w!("\\GPU Engine(*)\\Utilization Percentage"),
                0,
                &mut counter,
            ) != 0
            {
                PdhCloseQuery(query);
                return None;
            }
            PdhCollectQueryData(query);
            Some(Self {
                query,
                counter,
                prefix: format!(
                    "pid_{}_luid_0x{:08x}_0x{:08x}_",
                    std::process::id(),
                    luid >> 32,
                    luid as u32
                ),
                valid: 0,
                positive: 0,
                maximum: 0.0,
            })
        }
    }
    pub fn sample(&mut self) {
        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return;
            }
            let (mut size, mut count) = (0, 0);
            if PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_DOUBLE,
                &mut size,
                &mut count,
                None,
            ) != PDH_MORE_DATA
                || size > 4 * 1024 * 1024
            {
                return;
            }
            let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
            let items = buffer.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
            if PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_DOUBLE,
                &mut size,
                &mut count,
                Some(items),
            ) != 0
                || count as usize
                    > buffer.len() * 8 / std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>()
            {
                return;
            }
            let (start, end) = (
                buffer.as_ptr() as usize,
                buffer.as_ptr() as usize + buffer.len() * 8,
            );
            let mut maximum: Option<f64> = None;
            for item in std::slice::from_raw_parts(items, count as usize) {
                if item.FmtValue.CStatus > 1 {
                    continue;
                }
                let address = item.szName.0 as usize;
                if address < start || address >= end || !address.is_multiple_of(2) {
                    continue;
                }
                let text = std::slice::from_raw_parts(item.szName.0, (end - address) / 2);
                let Some(len) = text.iter().position(|c| *c == 0) else {
                    continue;
                };
                let name = String::from_utf16_lossy(&text[..len]);
                if !matches_engine(&name, &self.prefix) {
                    continue;
                }
                let value = item.FmtValue.Anonymous.doubleValue;
                if value.is_finite() && value >= 0.0 {
                    maximum = Some(maximum.unwrap_or(0.0).max(value));
                }
            }
            if let Some(value) = maximum {
                self.valid += 1;
                self.positive += usize::from(value > 0.01);
                self.maximum = self.maximum.max(value);
            }
        }
    }
    pub fn report(&self, frames: u32) -> Value {
        json!({"source":"Windows PDH GPU Engine", "scope":"this process, selected adapter LUID, engtype_VideoDecode only",
            "valid_samples":self.valid, "active_samples":self.positive, "peak_percent":self.maximum,
            "hardware_decode_observed": if frames>=60 && self.positive>=2 {Some(true)} else {None::<bool>},
            "note":"Missing/zero activity is inconclusive. GPU output alone is not hardware decode proof. Internal GPU copy counts are unknown."})
    }
}
fn matches_engine(name: &str, prefix: &str) -> bool {
    name.starts_with(prefix) && name.ends_with("engtype_VideoDecode")
}
impl Drop for DecodeCounter {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.query);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rendering_other_processes_and_other_adapters_cannot_prove_decode() {
        let prefix = "pid_12_luid_0x00000000_0x00000123_";
        assert!(matches_engine(
            "pid_12_luid_0x00000000_0x00000123_phys_0_eng_2_engtype_VideoDecode",
            prefix
        ));
        for name in [
            "pid_13_luid_0x00000000_0x00000123_phys_0_eng_2_engtype_VideoDecode",
            "pid_12_luid_0x00000000_0x00000124_phys_0_eng_2_engtype_VideoDecode",
            "pid_12_luid_0x00000000_0x00000123_phys_0_eng_0_engtype_3D",
            "pid_12_luid_0x00000000_0x00000123_phys_0_eng_1_engtype_VideoProcessing",
        ] {
            assert!(!matches_engine(name, prefix));
        }
    }
}
