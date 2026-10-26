//! Shared UTF-16 text decoding; native message/key interpretation lives in platform adapters.
#[derive(Default)]
pub struct TextDecoder(Option<u16>);
impl TextDecoder {
    pub fn push(&mut self, unit: u16) -> Option<char> {
        if (0xD800..=0xDBFF).contains(&unit) {
            self.0 = Some(unit);
            return None;
        }
        let high = self.0.take();
        let code = if (0xDC00..=0xDFFF).contains(&unit) {
            let high = high?;
            0x10000 + ((u32::from(high) - 0xD800) << 10) + u32::from(unit) - 0xDC00
        } else {
            u32::from(unit)
        };
        char::from_u32(code).filter(|c| !c.is_control())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_text_combines_surrogates_and_omits_control_duplicates() {
        let mut d = TextDecoder::default();
        assert_eq!(d.push(0xD83E), None);
        assert_eq!(d.push(0xDD80), Some('🦀'));
        assert_eq!(d.push(9), None);
        assert_eq!(d.push(22), None); // Ctrl+V is a key, not text
        assert_eq!(d.push('ő' as u16), Some('ő'));
        assert_eq!(d.push(0xDC00), None);
    }
}
