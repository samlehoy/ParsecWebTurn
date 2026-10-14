# Offline compressed video fixture

`synthetic-720p-hevc.bin` is the synthetic 8-bit HEVC fixture from lowlat,
revision `5f8db5689e55de0d396b0520824caa9fc053dd86`, originally
`crates/decode/tests/data/synthetic-720p-hevc.bin`.
It contains 120 length-prefixed access units. The file is copied unchanged.
Copyright (c) 2026 Nguyen Duy; MIT license in `../LICENSE-lowlat`.

It is used only by the offline Intel GPU decode probe and is not packaged in
the normal client. The H.264 probe reuses our existing synthetic fixture.
