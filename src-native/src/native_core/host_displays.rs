//! Bounded stock-host display list; IDs are opaque, never desktop coordinates.
use anyhow::{ensure, Result};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Output {
    pub id: String,
    pub name: String,
    #[serde(rename = "adapterName", default)]
    pub adapter: String,
}
#[derive(Default)]
pub struct Displays {
    pub outputs: Vec<Output>,
    pub current: Option<String>,
    pub loading: bool,
    pub hid_mouse: bool,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Primary<'a> {
    Waiting,
    Ready,
    Select(&'a str),
}
impl Displays {
    pub fn primary(&self, windows_host: bool, current: Option<&str>) -> Primary<'_> {
        if !windows_host || self.loading {
            return Primary::Waiting;
        }
        let Some(first) = self.outputs.first() else {
            return Primary::Waiting;
        };
        if current == Some(first.id.as_str()) {
            Primary::Ready
        } else {
            Primary::Select(&first.id)
        }
    }

    pub fn validate_selection(&self, id: &str, hid_mouse: bool) -> Result<()> {
        ensure!(
            !hid_mouse,
            "Display switching is unavailable with HID mouse input"
        );
        ensure!(
            !self.loading && self.outputs.iter().any(|o| o.id == id),
            "Host display is unavailable; refresh the list"
        );
        Ok(())
    }
}
pub fn parse(bytes: &[u8]) -> Result<Vec<Output>> {
    ensure!(bytes.len() <= 65536, "Host display list exceeds limit");
    let outputs: Vec<Output> =
        serde_json::from_slice(bytes).map_err(|_| anyhow::anyhow!("Invalid host display list"))?;
    ensure!(outputs.len() <= 64, "Too many host displays");
    for (index, output) in outputs.iter().enumerate() {
        ensure!(
            !output.id.is_empty()
                && [&output.id, &output.name, &output.adapter]
                    .iter()
                    .all(|s| s.len() <= 1024 && !s.chars().any(char::is_control))
                && !outputs[..index].iter().any(|o| o.id == output.id),
            "Invalid or duplicate host display"
        );
    }
    Ok(outputs)
}
pub fn query() -> Result<bytes::Bytes> {
    crate::control::text(17, 10, "")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_primary_selection_waits_for_both_reports_and_keeps_dxgi_order() {
        let mut displays = Displays::default();
        assert_eq!(displays.primary(true, Some("secondary")), Primary::Waiting);
        displays.outputs =
            parse(br#"[{"id":"z-primary","name":"Main"},{"id":"a-secondary","name":"Other"}]"#)
                .unwrap();
        assert_eq!(displays.primary(false, None), Primary::Waiting);
        assert_eq!(
            displays.primary(true, Some("a-secondary")),
            Primary::Select("z-primary")
        );
        assert_eq!(displays.primary(true, Some("z-primary")), Primary::Ready);
        displays.loading = true;
        assert_eq!(displays.primary(true, Some("z-primary")), Primary::Waiting);
    }
    #[test]
    fn hid_and_stale_lists_cannot_select_an_output() {
        let mut state = Displays {
            outputs: parse(br#"[{"id":"secondary","name":"Display"}]"#).unwrap(),
            ..Default::default()
        };
        assert!(state.validate_selection("secondary", false).is_ok());
        assert!(state.validate_selection("secondary", true).is_err());
        assert!(state.validate_selection("unknown", false).is_err());
        state.loading = true;
        assert!(state.validate_selection("secondary", false).is_err());
    }
    #[test]
    fn host_outputs_are_bounded_unique_and_preserve_opaque_ids() {
        let list = br#"[{"id":"physical","name":"Display 1","adapterName":"GPU"},{"id":"virtual","name":"Virtual display"}]"#;
        assert_eq!(parse(list).unwrap()[1].id, "virtual");
        for list in [
            r#"[{"id":"","name":"bad"}]"#,
            r#"[{"id":"x","name":"a"},{"id":"x","name":"b"}]"#,
            r#"[{"id":"x","name":"a\u0000b"}]"#,
            r#"{"id":"x","name":"a"}"#,
        ] {
            assert!(parse(list.as_bytes()).is_err());
        }
        assert!(parse(&vec![b' '; 65537]).is_err());
    }
}
