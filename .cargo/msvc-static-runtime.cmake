# Upstream Opus build option, not a codec/source modification.
# Only selected for the x86_64-pc-windows-msvc Cargo target.
set(OPUS_STATIC_RUNTIME ON CACHE BOOL "Match Rust's static MSVC runtime" FORCE)
