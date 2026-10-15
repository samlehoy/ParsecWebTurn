# Native BUD transport adapter

Session, signaling and compressed media parsing for the explicit experimental
BUD setting. WebRTC remains the default transport. This crate has no decoder,
renderer, GPU exporter, audio device or account UI; the application supplies them.

Direct UDP, automatic UDP relay fallback and Force TURN are retained. TURN TCP/TLS
belong to the separate WebRTC transport. Credentials and lifecycle stay isolated;
there is no silent fallback from BUD to WebRTC. Compressed media enters the same
native decoder/audio pipeline as WebRTC, without exporting decoded pixels.

The transport-only lowlat-client and lowlat-net subsets are pinned and documented
in vendor/*/PARSECWEBTURN.md. Their MIT licenses are retained. Regression tests
exercise encrypted direct and UDP TURN paths using synthetic local credentials.
One-off GPU investigations and their optional dependencies have been removed.
