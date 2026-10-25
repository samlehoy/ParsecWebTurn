# Offline HEVC regression fixture

`synthetic-720p-hevc.bin` is copied unchanged from lowlat revision
`5f8db5689e55de0d396b0520824caa9fc053dd86`, originally
`crates/decode/tests/data/synthetic-720p-hevc.bin`.
It contains 120 length-prefixed HEVC Main 8-bit 4:2:0 access units.
Copyright (c) 2026 Nguyen Duy; MIT license in `../LICENSE-lowlat`.

Only regression tests include this file, to verify bounded presentation leases
against an actual parsed HEVC reference-picture buffer. Normal client execution
does not include or play it. Standalone comparison executables have been retired.
