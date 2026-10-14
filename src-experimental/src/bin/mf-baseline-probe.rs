// Share the offline probe source without rustfmt traversing it using this
// crate's 2024 edition. Its original standalone crate uses the 2021 edition.
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../tests/latency-probes/src/bin/decoder-wait.rs"
));
