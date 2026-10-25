# EvoParseq development references

EvoParseq is an independent implementation for compatible Parsec hosts. External
projects are research inputs, not compatibility guarantees.

## lowlat

[nomi-san/lowlat](https://github.com/nomi-san/lowlat), MIT, provides the pinned
protocol, parser, GPU-driver and BUD transport components identified in the local
`EVOPARSEQ.md` provenance notices. Original copyrights and licenses are retained.
Only the components required by this application are included; no lowlat SDK,
host, capture stack or second application/session implementation is embedded.

The GPU driver/parser reference is revision
`5f8db5689e55de0d396b0520824caa9fc053dd86`. TURN stream IO improvements also use
reviewed upstream revisions `befd5a29c7a331111f11378c0b5623de788e9769`,
`d5d8d9ed4b1208f00de9fe8f90286919b15a0240` and
`b174dc6ac8d4923f45abe0c3a0a0a41fc0b76c49`. The receive-only cursor cache
reference is `e548d48c81ae50937940b2b47f700557cd9d50a4`.

## Native APIs and protocols

- Microsoft documented D3D11 video, DXGI, Windows Presentation API, WASAPI,
  Win32 input and CurrentUser DPAPI.
- W3C/IETF WebRTC, ICE, STUN, TURN, DTLS and SCTP; local third-party patches
  retain their upstream licenses and document changes in `EVOPARSEQ.md`.
- Vendor codec documentation and runtime capability queries. Device vendor
  names alone never establish an implemented decoder/output format.

Refer to [native architecture](native-core.md) and the [root README](../README.md)
for implemented behavior and the distinction between live and synthetic testing.
