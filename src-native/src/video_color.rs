//! Translate Media Foundation metadata into D3D11's differently numbered range.
//! Values here describe samples; no pixel conversion or copying is performed.
pub fn processor_range(mf_nominal: Option<u32>) -> u32 {
    match mf_nominal {
        Some(1) => 2, // MF 0-255 -> D3D11 0-255.
        _ => 1,       // MF 16-235 or unspecified -> D3D11 16-235.
    }
}

pub fn processor_color(matrix: Option<u32>, mf_nominal: Option<u32>) -> u32 {
    (if matrix.unwrap_or(1) == 1 { 4 } else { 0 }) | (processor_range(mf_nominal) << 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mf_and_d3d11_range_enums_are_opposite_and_unknown_is_limited() {
        assert_eq!(processor_color(Some(1), Some(1)), 4 | (2 << 4));
        for nominal in [None, Some(0), Some(2)] {
            assert_eq!(processor_color(Some(1), nominal), 4 | (1 << 4));
        }
        assert_eq!(processor_color(Some(2), Some(1)), 2 << 4);
        assert_eq!(processor_color(None, None), 4 | (1 << 4));
    }
}
