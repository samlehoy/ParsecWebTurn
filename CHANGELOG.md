# Changelog

User-facing changes are listed below. Native versions marked **dev** are development
builds, not published stable releases.

## Unreleased

- **Two-way text clipboard:** forward native clipboard changes, stream focus restoration and paste-key requests to the original core. Preserve clipboard preference/host permissions, avoid echoing incoming host copies, and keep unavailable reads distinct from copied empty text. Live client-to-host validation is pending.
- **Host timing labels:** correct the reversed encode-duration and video-frame-interval labels. Values remain in milliseconds without rescaling; audio interval anomalies remain under investigation.
- **Detailed host timing (experimental):** automatically request video/audio timing while Connection stats is visible or event logging is enabled. Show reported processing durations, video capture/encode segments and frame/block intervals with latest/average/peak values, report counts and freshness. Bounded collectors bypass application control queues in BUD and WebRTC; malformed or unavailable diagnostics do not fail the stream. No separate switch or per-frame file logging. Live BUD report reception observed; WebRTC validation and audio interval anomalies remain under investigation.
- **Multichannel audio (experimental, BUD; synthetic tests only):** optional automatic 5.1/7.1 requests based on the Windows output layout, per-packet Opus/PCM format handling, native multichannel playback, and actual channel information in Connection stats. Stereo remains the default and the host may fall back to stereo. Validation covers synthetic Opus/PCM tests and silent WASAPI output only; live host-to-client operation, channel ordering and audible surround playback remain unverified. WebRTC retains stereo.
- **Relative mouse transition:** hide the local cursor before Raw Input registration and confinement, while retaining cursor restoration if acquisition fails. This narrows the visible transition; the reported intermittent flash still needs live verification.

## 0.9.4-dev.5 - Experimental USB HID input (2026-10-20)

Changes since v0.9.4-dev.4.

**USB HID input, BUD, vendor-specific HEVC decoding, Zero copy, 10-bit HEVC
and the Audio options remain experimental.** For stability-first use, select
WebRTC with H.264, leave experimental options disabled, or use the published
stable v0.9.3 release. HID input requires BUD and USB/Yubikey passthrough on
the stock host; compatibility depends on the host and application.

- **Experimental USB HID mouse:** use the stock host's USB/Yubikey passthrough with BUD for absolute desktop positioning, native relative camera movement, five buttons, dragging and vertical/horizontal scrolling. Relative capture follows the host cursor state and hides the local pointer. One physical or virtual display is supported.
- **Experimental USB HID keyboard:** forward physical key presses, releases and modifiers through the same host USB path. Matching active layouts, including Hungarian on both computers, are supported; the host layout is not changed automatically. Simultaneous keys use a bitmap without a six-key array limit.
- **Simultaneous HID input:** mouse and keyboard share one USB device lifecycle and bounded transport queues, with separate reports and fair scheduling. Buttons keep the same owner across desktop/camera mode changes. Ordinary Parsec forwarding is disabled independently for each selected HID input, preventing duplicate input.
- **New Input settings tab:** select HID mouse and keyboard independently, save the preferences and reconnect. Both options require BUD and are disabled by default. Local menus remain usable; focus loss and disconnect release held input and capture. Only the usual EXE and settings.json are needed, without a host modification or additional launcher.
- **HID operational logging:** the existing optional client-events.jsonl records activation, state changes and setup/transfer/queue failures, without continuous mouse movement, coordinates, clicks, key history, typed text or USB payloads.

## 0.9.4-dev.4 - Relay IO improvements and 10-bit SDR output (2026-10-17)

Changes since v0.9.4-dev.3.

**BUD, vendor-specific HEVC decoding, Zero copy, 10-bit HEVC and the Audio
options remain experimental.** For stability-first use, select WebRTC with H.264,
leave experimental options disabled, or use the published stable v0.9.3 release.

- **Native BUD TCP/TLS IO:** relay sockets use the existing platform wake and native socket wait, with overlapped IOCP on Windows, bounded reusable storage and cancellation. Queued TURN frames drain from reused staging storage without a batching delay; received frames are parsed in place. Direct BUD remains UDP, and TLS retains certificate and hostname verification through rustls.
- **BUD TLS backpressure:** retain pending frames until both TURN framing and encrypted TLS output have drained to the socket. This prevents new protocol output from building up behind earlier TLS records and preserves write deadlines during slow sends.
- **More efficient WebRTC TURN streams:** reuse a bounded receive buffer for plain TURN/TCP and write ChannelData with its padding as one framed write on TCP/TLS. TLS retains its existing receive path. No batching delay is added; UDP routing, ICE selection and SCTP behavior are unchanged.
- **Experimental 10-bit RGB SDR output:** actual Main 10/P010 decoder output can now use a 10-bit RGB swap chain when the GPU and driver support conversion and presentation. The existing GPU conversion writes directly into the output buffer, without an additional decoded-frame copy or conversion pass. Unsupported output falls back to 8-bit RGB. This is SDR, not HDR, and does not establish physical display or scanout depth.
- **Clearer video precision status:** Connection stats and optional event logging distinguish decoded color depth from application RGB output depth and report output fallback. Live Main 10 with 10-bit RGB output is currently verified with NVIDIA host encoding and AMD Radeon 780M client decoding; the tested AMD host path sent 8-bit HEVC. Original-surface Zero copy remains 8-bit only, and 4:4:4 remains unsupported.
- **Leaner native components and acknowledgements:** remove unused direct H.264 decoder code and its unused GPU protocol-core dependency; H.264 continues to use Media Foundation on all GPU vendors. The README now explicitly credits lowlat inspiration and reused components while clarifying that ParsecWebTurn is an independent project.

## 0.9.4-dev.3 - BUD TCP/TLS relays and audio controls (2026-10-16)

Changes since v0.9.4-dev.2.

**BUD, vendor-specific HEVC decoding, Zero copy, 10-bit HEVC and the new Audio
options are experimental.** For stability-first use, select WebRTC with H.264,
leave experimental options disabled, or use the published stable v0.9.3 release.

- **Experimental BUD over TURN TCP/TLS:** use the existing TURN URLs for BUD as well as WebRTC, with UDP preferred, then TLS and plain TCP (port 80 preferred among TCP URLs). Failed startup attempts can try the next configured relay; direct BUD remains UDP. TLS checks the server identity. BUD TURN/TLS playback has been confirmed with a stock host using AMD AMF HEVC and Direct D3D11 HEVC.
- **Reliable TCP/TLS startup and responsive IO:** handle pending nonblocking TCP connections correctly, disable Nagle before sending TLS/application data and retain partial writes under backpressure. The event-driven carrier uses bounded reusable buffers and immediate input wakeups without a batching delay. Event logging identifies failed transport stages and IO error codes without recording credentials or packet contents.

- **Experimental Audio settings:** add a separate Audio tab with optional minimum/maximum playback buffering, supported low-period WASAPI initialization with fallback, bounded fresh PCM playback, and BUD-only raw PCM permission. Automatic playback and Opus remain the defaults. Raw requires host support and can still fall back to Opus; WebRTC audio remains Opus.
- **Audio visibility:** show actual audio codec, local output buffer and limits, device capacity, decoded PCM backlog and underruns in Connection stats. Operational logging includes audio configuration and playback-error counters without media payloads.

- **HEVC-dependent video options:** disable the Zero copy checkbox while Prefer H.265 / HEVC is off, matching the 10-bit HEVC option. Turning HEVC back on restores access to the saved preference.
- **BUD Main 10 startup fix:** remove an obsolete 8-bit-only parser check that disconnected BUD when Main 10 video arrived. Advertised Main 10 is passed to the existing MF/P010 GPU path; host/driver compatibility still needs testing. 4:4:4 remains unsupported.

## 0.9.4-dev.2 - Color range fixes and event logging (2026-10-16)

Changes since v0.9.4-dev.1.

**BUD, vendor-specific HEVC decoding, Zero copy and 10-bit HEVC are experimental.**
For stability-first use, select WebRTC with H.264 and leave experimental options
disabled, or use the published stable v0.9.3 release.

- **Correct video black and white levels:** translate limited/full-range metadata correctly between Media Foundation and D3D11, including vendor HEVC output. This fixes lifted blacks and grey-looking whites in normal GPU rendering. Full-range support is declared consistently at startup and in later encoder requests.
- **Experimental 10-bit HEVC:** an optional Video setting requests Main 10 with WebRTC or BUD when the selected GPU exposes the required P010 decode/conversion capabilities. Later encoder and keyframe requests preserve the selected flags. Main 10 uses the existing GPU Media Foundation path and conversion pass; RGB display output remains 8-bit, and original-surface Zero copy remains 8-bit only. Host and driver support are required: the request does not guarantee a 10-bit stream. Connection stats shows the actual decoded color depth. 4:4:4 remains unsupported.
- **Optional operational event log:** enable event logging in the Network tab to record connection and route changes, transport failures, decoder recovery/errors and profile-save failures beside settings.json. UTF-8 JSON Lines records have UTC timestamps; background file writes use a bounded queue and approximately 5 MiB rotation with one previous file. Credentials and media payloads are excluded. Logging is disabled by default and applies immediately after saving.
- **Readable settings and statistics:** fix clipped event-log help in Settings and size the Connection stats explanation to its wrapped text, including the longer BUD description and narrower windows.
- **Clearer core provenance:** README documentation explains that the embedded proprietary Parsec WASM core remains unmodified and that Rust supplies the native framework rather than a rewrite of that core.

## 0.9.4-dev.1 - Experimental BUD and GPU video paths (2026-10-15)

Changes since the stable v0.9.3 release.

**BUD, vendor-specific HEVC decoding and Zero copy are experimental.** For
stability-first use, select WebRTC with H.264 and leave the experimental options
disabled, or use the published stable v0.9.3 release. H.264 remains the default
and uses Media Foundation on every GPU vendor.

- **Experimental BUD transport:** enable Experimental BUD in Settings to use the native UDP protocol with a regular Parsec host. Direct UDP, automatic fallback to a configured UDP TURN relay and Force TURN are supported. WebRTC remains the default and retains TURN over UDP, TCP and TLS; BUD supports UDP only and does not silently switch to WebRTC.
- **Experimental vendor-specific HEVC decoding:** when Prefer H.265 / HEVC is enabled, normal 8-bit HEVC playback uses Direct D3D11 on NVIDIA, AMF on AMD and VPL on Intel. These paths keep decoder output on the GPU and feed the existing presentation path directly. The existing GPU Media Foundation fallback/recovery remains available when a vendor path cannot be used.
- **Experimental Zero copy presentation:** the Video tab can select original NV12 decoder-surface presentation through Direct D3D11 and Windows Presentation API on supported NVIDIA, AMD and Intel GPUs. This removes the application's conversion/output pass and introduces no intermediate decoded-frame copy or CPU pixel readback. Windows 11 and compatible hardware/drivers are required; internal Windows/driver copies and direct scanout are not guaranteed.
- **More reliable HEVC startup and recovery:** handle short arrival bursts without exhausting presentation surfaces or unnecessarily abandoning the working Intel decoder. Original-surface presentation preserves the picture's aspect ratio and can recover to normal GPU rendering when unavailable.
- **BUD connection statistics:** show native RTT, decoded FPS, incoming/outgoing traffic, direct or relay route, host encode time when available and BUD delivery counters. Irrelevant WebRTC ICE/DTLS fields are hidden in BUD mode, and the last valid host encode time no longer flickers between reports.
- **Network and Video settings tabs:** connection servers, credentials, routing and transport settings are separated from GPU, codec and presentation choices. Both tabs share Save settings and retain unsaved edits when switching tabs.
- **Clearer playback status:** Connection stats uses shorter Zero copy, decoder fallback and playback-error descriptions while retaining detailed diagnostic information separately.
- **Safer host launch selection:** choosing the unsupported native-client launch option now explains that the web launch option is required instead of crashing the application. The web launch option is also used when BUD is selected in Settings.
- **Less input and shared-lock overhead:** reuse bounded input-event storage, pass native input arguments without temporary vectors and copy queued control messages outside the shared connection lock. Event ordering, immediate input wakeups and WebRTC reliability remain unchanged.

## 0.9.3 - Native Windows client (2026-10-14)

Changes since the previous stable release, v0.7.0.

- **Native Windows client:** the Rust client replaces Tauri/WebView2. Run a single portable EXE without a browser runtime. The embedded Parsec core is precompiled, so it needs no download or compilation at startup.
- **H.265 / HEVC playback:** enable Prefer H.265 / HEVC to request an 8-bit stream. H.264 remains the default and fallback. Playback uses Windows Media Foundation and D3D11; HEVC requires compatible host, GPU, driver and Windows decoder support.
- **Video GPU selection:** choose the adapter for stream decoding and rendering on multi-GPU systems, or keep Automatic. Virtual display-only adapters no longer appear as duplicate GPUs.
- **More responsive controls:** input wakes processing immediately. Native relative mouse input and cursor warp handle desktop drags correctly; immersive Mouse, Keyboard and Both modes capture the selected input while the app has focus. Custom Parsec hotkeys are respected.
- **Flexible connection routing:** use Cloudflare or custom STUN/TURN servers over UDP, TCP or TLS. STUN-only excludes relays; Force TURN relay keeps the connection on a relay even when a direct route is available, including over a VPN.
- **More reliable TURN connections:** forced relay setup signals readiness without waiting for every transport endpoint. TURN channel bindings renew before the five-minute boundary, addressing the reported ExpressTURN interruption. Multiple server addresses can be entered on separate lines.
- **Compatible overlay and fullscreen:** software drawing is the default for the interface, resolving the reported Intel overlay flicker while streamed video retains its GPU path. The native menu hides in fullscreen.
- **Reliable session controls:** overlay disconnect returns to the computer list. Settings opens and closes more reliably; coordinated control painting and scroll limits reduce redraw flicker.
- **Stream efficiency and recovery:** reuse media buffers and GPU video views, shorten shared locks, disable extra picture processing and request low-latency decoding. Bounded keyframe recovery helps restart video after temporary decoder errors or overload. Results depend on the driver and host.
- **Native sound and saved preferences:** Opus playback uses native Windows audio with device-supported low-latency output and fallback. Sign-in and Parsec preferences persist in an encrypted Windows-user profile; existing server settings remain compatible.
- **Connection stats and About:** separate native statistics show actual direct/TURN routing, relay details, traffic, RTT, decoded video format, decoder, GPU, audio format and app resource usage. About compares the embedded core version with Parsec's public version without installing a new core.
- **Stable update notifications:** background checks and Help > Check for updates show stable releases and their notes. Download the ZIP through your browser and replace the EXE manually while keeping your settings.

## 0.9.3-dev.1 - Connection and input improvements (2026-10-12)

- **Force TURN relay:** a new Settings checkbox keeps the next connection on a TURN relay even when a direct route is available, including over a VPN. Conflicting STUN-only settings and missing custom TURN servers are checked before connecting.
- **TURN transport preference:** relay connections prefer UDP, with TLS and then plain TCP available as timed fallbacks when the preferred route has not connected.
- **Working immersive input:** Mouse confines the pointer to the client window, Keyboard forwards reserved shortcuts while the client has focus, and Both combines them. Capture is released on focus loss and disconnect; Windows security shortcuts remain controlled by Windows.
- **Smoother relative mouse control:** host cursor signals now activate native Raw Input correctly, including during desktop drags. Changing direction no longer requires moving back from outside the client. Cursor Warp restores the host cursor position when leaving relative mode and respects its overlay setting.
- **More compatible overlay:** software drawing is now the default for the interface, resolving the reported Intel overlay flicker. Video decoding and presentation retain their GPU path without adding decoded-frame CPU readback.
- **Reliable disconnect:** disconnecting from the overlay returns to the computer list instead of freezing on a black screen. Callbacks from an ended stream cannot reveal its old video surface.
- **Cleaner fullscreen:** the Settings, Connection stats and Help menu disappears in fullscreen and returns in windowed mode.
- **Custom hotkeys:** Parsec hotkey assignments take priority when they conflict with native F8/F11, Settings or Connection stats shortcuts. Hotkey recording and saving remain handled by the original core.
- **Less allocation and lock contention:** audio and video reuse bounded input buffers, and media hand-off holds the shared connection lock for less time.
- **Video recovery:** after a temporary decoder error or input overload, the client can request a fresh keyframe and restart decoding. Recovery has bounded retries and depends on the host supplying the requested keyframe.
- **Low-latency HEVC:** the 8-bit HEVC decoder also requests low-latency operation through Windows transform attributes when its codec API does not accept the existing request. The benefit depends on the decoder and driver.

## 0.9.2-dev.1 - H.265 / HEVC video (2026-10-11)

- **H.265 / HEVC support:** enable **Prefer H.265 / HEVC** in Settings to request an 8-bit HEVC stream on your next connection. The client checks decoder support on your selected GPU first; H.264 remains the default and is used when HEVC is unavailable or the host chooses H.264.
- **Native HEVC playback:** HEVC uses the Windows decoder and the existing D3D11 video path. Successful video playback has been verified with a Parsec host.
- **HEVC details in Connection stats:** see the received codec and profile, decoder, resolution and graphics adapter. These describe the stream actually received, rather than just your selected preference.

## 0.9.1-dev.1 - Faster startup and more responsive controls (2026-10-10)

- Mouse and keyboard input now wakes processing immediately instead of waiting for the next regular tick. Local testing showed substantially shorter input processing times; actual responsiveness depends on your system and connection.
- The bundled Parsec core is precompiled during the build, so the app no longer compiles it at startup.
- The GPU selector filters out virtual display-only adapters that could appear as duplicate graphics cards.
- The statistics window now has the title **Connection stats** in its Windows title bar.

This first published native preview also includes the connection settings, live statistics and other native-client features listed under 0.9.0 and 0.8.0 below.

## 0.9.0 - More connection options and live stats (dev, 2026-10-10)

### New

- **Video GPU selector:** choose the graphics adapter used for native stream decoding and rendering in Settings. The choice is saved and applies to your next connection; Automatic keeps the Windows default.
- **Choose your own connection servers.** Configure custom STUN servers and TURN relays, including Cloudflare, coturn, eturnal and ExpressTURN. TURN connections support UDP, TCP and encrypted TLS, including servers on port 443.
- **STUN-only mode for LAN/VPN use.** Discover a direct connection without offering a TURN relay. Leave the STUN field empty to use Parsec's default server.
- **Keep your previous server settings.** The native client can read the earlier WebView2 client's `settings.json`, including Windows-protected credentials.
- **A separate Connection stats window.** See connection delay, traffic, video FPS and resolution, audio format, encryption status and app CPU/GPU usage. The window identifies confirmed direct or TURN connections and the relay server when available. Unavailable details are left unknown or omitted.
- **Client-side decoder details.** See the video decoder, renderer and graphics adapter. Decoder-specific GPU surfaces are shown as evidence of likely hardware decoding, rather than a guarantee.
- **Help menu with version information.** Check both the ParsecWebTurn version and the embedded Parsec core version.

### Improved

- **More responsive input:** mouse and keyboard events now wake their processing loop immediately instead of waiting for its next regular tick. The regular interface callback and video presentation keep their existing pacing. Actual latency improvement depends on system load and the Parsec core.
- **Faster startup:** the bundled Parsec core is compiled during the build and loaded directly when you open the app. No download or compilation is needed at startup.
- The Video GPU selector excludes virtual display-only adapters that can appear under the same name as a real GPU. Separate real GPUs remain selectable, even when their names match.
- Disable the video driver's automatic extra picture processing and reuse GPU video views and stream-format details. Cached views are released when the stream changes or the window is resized. Presentation queue behavior is unchanged.
- A clearer, Parsec-inspired **Settings** page with descriptions, provider-specific fields and dark controls.
- Fixed overlapping text, clipped buttons and scrolling/repainting problems in Settings. Improved the stats window's resizing, scrolling and handling of ended sessions.
- Sign-in and Parsec preferences now save correctly between launches.
- Disconnecting from the overlay or closing the app sends a disconnect notification to the host instead of leaving it waiting for a timeout.
- The window keeps the **ParsecWebTurn** name so it is clearly distinct from the official client.
- Windows gives the audio and video workers multimedia scheduling priority when available. Audio output requests the shortest device-supported playback period, with automatic fallback to the previous audio path. These changes target local playback delay; the benefit depends on your device and workload.
- Removed obsolete WebView2 files and kept diagnostic tools out of the normal app.

### Before you use this build

- This is still a Windows native development build. Vulkan, D3D12, Linux and Android support are not included.
- Audio uses Opus. Selecting RAW in the embedded interface did not produce a RAW stream in testing; RAW playback is not supported.
- Enabling TURN can result in a relay being selected even when a direct path is possible. Use **STUN only** if you want to exclude relays.
- Client-to-host clipboard paste remains a known issue in the tested Parsec session; it also reproduced in standalone Edge.

## 0.8.0 - Native Rust client (dev)

- Replace the WebView2 launcher with the native Rust client using the pinned Parsec WASM core (150-104a).
- Embed the core in one executable; no separate WASM file, launcher script or WebView2 runtime is required.
- Keep sign-in and client preferences in an encrypted, current-Windows-user profile across restarts. Previous Tauri configuration and profiles remain untouched.
- Present GPU-resident decoded video through the native Windows graphics path; add native Opus/WASAPI audio, keyboard, pointer and the original Parsec overlay. Relative pointer capture remains controlled by the overlay.
- Enable legacy RSA-1024 host identity compatibility while retaining signature and certificate fingerprint verification.
- Remove the prototype's forced Cloudflare STUN setting and exclude diagnostic commands, synthetic fixtures and test-only readback from the normal executable.

Custom server settings and native connection statistics were added in 0.9.0 above.

## [0.7.0] - 2026-10-02

### Added

- Separate STUN and custom TURN address fields with automatic migration of existing mixed address lists.
- Add **STUN only - no TURN relay** for direct connections. Cloudflare mode can use its public STUN endpoint without an API token or TURN credential generation. Custom STUN addresses also work alongside Cloudflare TURN.
- Show reported WebRTC DTLS state, version, cipher suite and SRTP protection profile in Connection stats.
- Show CPU usage for the application and its WebView2 processes, plus WebView2 GPU engine and video-decode utilization when available.
- Show detected audio codec, sample rate and channel count, retaining these values during silence. The sender's configured bitrate remains unknown when the receiving APIs do not expose it.

### Fixed

- Configure clipboard-read permission for the exact Parsec web origin using the documented WebView2 profile API, fixing empty local Clipboard API reads.

### Known limitations

- Client-to-host paste still fails in the tested Parsec session and also reproduces in standalone Edge. The clipboard permission change does not resolve Parsec's end-to-end clipboard synchronization.
- With TURN enabled, native ICE selects the connection route; a reachable direct path does not guarantee that it selects a direct connection. Use STUN-only when TURN must be excluded.

## [0.6.0] - 2026-10-01

### Added

- Background GitHub stable-release notifications and App > Check for updates.
- Native update window with release notes and a direct link to the official Windows ZIP, opened in the default browser only after a click.
- Bounded metadata-only HTTPS checks, stable numeric version comparison and exact repository ZIP link validation.

### Changed

- Hide detailed video statistics, including codec and resolution, when optional Media diagnostics are disabled; saved preference changes apply to their visibility immediately.
- Audit the runtime against public web specifications and documented platform APIs; record ongoing requirements in AGENTS.md and findings in docs/STANDARDS.md.
- Replace private Tauri frontend IPC access with the documented global core API; keep native capability restrictions.
- Make experimental Chromium Media diagnostics explicit opt-in, disabled for both new and legacy settings. Standard WebRTC/WebCodecs telemetry remains the baseline.
- Remove production Chromium launch switches and legacy vendor-prefixed WebRTC/statistics fallbacks.
- Preserve native WebCodecs callback dictionary conversion, inherited/frozen callbacks and callback capture semantics.
- Update notifications do not download or run executables, create a helper, replace the app or restart it. Updates are installed manually and follow the browser's organization policies.
- Correlate selected ICE endpoints with the ICE transport and gathered local/remote candidates to identify additional direct paths, including VPN connections. Positive TURN evidence retains priority; incomplete or ambiguous evidence stays unverified.
- Label confirmed direct routes as "Direct - no TURN"; VPN routing underneath WebRTC does not imply TURN use. Candidate addresses remain inside the WebView and are not included in telemetry.

## [0.5.0] - 2026-09-30

### Changed

- Replace the Go launcher and external Edge app window with a Rust/Tauri 2 application using Windows WebView2.
- Integrate settings, the Parsec window, developer tools and connection statistics into the same application.
- Embed the ICE override and UI in the executable; no extension directory or generated credential script is needed.
- Preserve v0.4.0 settings and CurrentUser DPAPI encryption. WebView2 uses a new profile and requires a fresh Parsec sign-in.
- Port Cloudflare requests, custom TURN, encrypted caching, validation and local ice.json fallback to Rust.
- Keep the direct/relay policy selected by Parsec; no forced relay mode.
- Replace ICE configuration spreading with a native WebIDL dictionary adapter, preserving inherited/getter fields, frozen inputs and native argument errors for constructors and setConfiguration.
- Validate STUN and TURN URI syntax separately and normalize case-insensitive scheme names.
- Build and validate the Windows distribution with Cargo and a committed lockfile.

### Added

- Live WebRTC traffic rates, connection RTT, direct/relay route and transport diagnostics.
- Show selected local/remote ICE candidate types; ambiguous candidate pairs remain unknown.
- Read Chromium Media codec, decoder, hardware decoding, profile and visible resolution, including stream reconfiguration.
- Native F11 fullscreen and Ctrl+Shift+W windowed recovery; block automatic web fullscreen and Escape locking.
- Proper App/View submenus and dark native windows.
- Video codec, decoder, FPS, resolution and packet-loss details when provided by the web client.
- Count actual WebCodecs decoded frames for data-channel video FPS; distinguish unavailable video packet loss from zero loss.
- Capture fullscreen recovery shortcuts before Parsec's handlers, including when the native menu is hidden.
- Mark peer-reflexive ICE paths as unverified instead of assuming direct routing.
- Recognize Chromium peer-reflexive TURN paths by the selected relay transport; show configured TURN usage, matched server and route evidence. Use the selected ICE transport pair when report selection is ambiguous.
- Restricted native command access: the remote Parsec page can submit bounded statistics but cannot read or change saved settings.

### Fixed

- Hide connection settings during automatic startup; show the ready form on first run, explicit settings launch or configuration/connection errors.
- Closing the Parsec window exits the application instead of reopening connection settings; internal window replacement during reconnect remains supported.

### Requirements

- Microsoft Edge WebView2 Runtime must be present. The runtime is separate from the Edge browser; the app does not silently install it.
- HEVC decoding and the previously reported long-session freeze remain separate issues; this migration does not claim to fix them.

## [0.4.0] - 2026-09-30

### Added

- Custom STUN/TURN provider configuration for standard services such as coturn and eturnal.
- Multiple server URLs with a common username/password; static and externally generated credentials, UDP/TCP/TLS endpoints and STUN-only configurations are supported.
- Windows DPAPI protection for saved custom TURN passwords.
- Provider switching retains Cloudflare and custom configuration without requiring re-entry of secrets.
- Custom provider, legacy migration, encrypted storage, no-network startup and native form-save tests.

### Changed

- Suppress Edge's automatic translation offer in the dedicated app profile while preserving other browser preferences.
- Modern settings window with a dark header, Parsec-inspired pink accents, a spaced connection card and inline validation.
- Provider-specific fields are shown only when relevant; custom mode requires no Cloudflare key, TTL or API request.
- Generalize startup errors to cover both providers.

### Security

- Custom credentials are encrypted in settings, but the generated browser injection necessarily contains the active TURN credential. For static custom accounts this credential is long-lived. Do not share the generated injection or profile.
- TURN REST shared secrets are not accepted as client passwords; generate the temporary username/password on your server and enter those values instead.

## [0.3.0] - 2026-09-29

### Added

- Optional DPAPI-encrypted TURN credential cache with conservative expiry checks and invalidation on Key ID, API token or TTL changes.
- Dedicated Edge profile detection and launcher locking to prevent stale configuration reuse and concurrent writes during startup.
- Go tests covering ICE/API validation, Windows DPAPI, credential caching, file replacement, profile locking and injection generation.
- JavaScript regression tests for construction, subsequent configuration updates, inheritance, duplicate injection and credential-free logging.

### Fixed

- ICE overrides now also apply to `RTCPeerConnection.setConfiguration()`.
- Invalid ICE entries and missing TURN authentication are rejected before Edge starts.
- Browser-blocked port 53 endpoints are removed from ICE configurations.
- TURN usernames/passwords are no longer printed to the console.
- Windows messages use real newlines; fallback errors retain both the API and fallback failure.
- Settings, cache and generated injection use temporary files with Windows replacement semantics.
- Local builds embed the same Common Controls v6 manifest as release builds.

### Changed

- Split the launcher into small files for settings, UI, DPAPI, Cloudflare, ICE, cache, files, profile, extension and Edge operations.
- Replace version-specific release workflows with one tag-triggered release workflow and a shared build/package script.
- Commit dependency checksums, pin the resource generator and store the icon locally; remove build-time icon downloads/conversion and `go mod tidy`.
- Add a TLS TURN port 443 fallback to the example configuration.
- Inject the executable version into the API User-Agent at build time.

### Compatibility

- Existing settings files still work; credential caching is disabled unless enabled in Settings.
- Credentials are refreshed at launch, not automatically within an open browser session. Close the dedicated Edge profile before relaunching and choose a TTL longer than your expected session.

## [0.2.1] - 2026-09-29

### Fixed

- Fixed `TTM_ADDTOOL failed` when opening the native settings window on affected Windows systems.
- Embedded a Windows application manifest enabling Microsoft Common Controls v6, which the Walk GUI toolkit requires for reliable tooltip/control initialization.


## [0.2.0] - 2026-09-29

### Added

- First-run native Windows settings dialog.
- Direct Cloudflare Realtime TURN credential generation using TURN Key ID + API token.
- Automatic fresh short-lived TURN username/password generation on every launch.
- Configurable credential TTL up to 172800 seconds / 48 hours.
- Windows DPAPI Current User protection for the stored Cloudflare API token.
- `--settings` and `/settings` options to reopen configuration.
- Local `ice.json` fallback when the Cloudflare API cannot be reached.
- Parsec icon embedded in the Windows executable.
- Explicit third-party/trademark notice for the icon.

### Changed

- `ice.json` is no longer required for normal use.
- Normal startup now obtains `iceServers` from Cloudflare before launching Edge.
- README expanded with Cloudflare setup and security model.

### Security

- The long-lived API token is not stored in plaintext.
- The token is encrypted using Windows DPAPI and bound to the current Windows user context.
- Short-lived TURN credentials may still appear in generated `extension/inject.js` for the duration of their validity.
- Backend-free mode assumes the local Windows user/device is trusted.

## [0.1.0] - 2026-09-29

### Added

- Initial public release of ParsecWebTurn.
- Portable Windows x64 launcher with no administrator rights required.
- Microsoft Edge app-mode launcher for `https://web.parsec.app/`.
- Automatic WebRTC ICE override at `document_start`.
- Cloudflare STUN/TURN support through external `ice.json` configuration.
- Separate local Edge profile stored beside the application.
- Example TURN configuration in `ice.example.json`.
- Manifest V3 helper extension scoped to `https://web.parsec.app/*`.
- GitHub Actions build workflow.
- MIT license.
