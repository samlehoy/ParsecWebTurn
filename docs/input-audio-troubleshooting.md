# Retired input and audio troubleshooting capture

The temporary `--trace-io` switch, JSONL sampler and test launcher have been
removed after the investigation. Current builds do not provide this capture.
Input capture fixes and the original shared WASAPI playback policy remain.

The investigation found that the affected VPN path also stuttered in the official
browser client; TURN substantially improved audio in the user’s tests. This does
not establish a universal VPN or WebRTC fault. Historical observations and their
limits are retained in [Host packet timing comparison](host-audio-transport-comparison.md).
