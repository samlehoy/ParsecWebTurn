# Updates and migration to EvoParseq

EvoParseq is an independent native Rust client for compatible, unmodified Parsec
hosts. It replaces the ParsecWebTurn client runtime; no Parsec WebAssembly core
is bundled or checked for updates.

## Existing ParsecWebTurn users

1. Close ParsecWebTurn. Keep your existing `settings.json` in its folder. A backup
   of the folder is recommended before replacing an application.
2. Extract the **transition ZIP** `ParsecWebTurn-v<VERSION>-win64.zip`, and replace
   the old `ParsecWebTurn.exe` with the file from that ZIP. This is the same
   EvoParseq executable, named for existing shortcuts. Alternatively, extract the
   normal EvoParseq ZIP beside your settings and launch `EvoParseq.exe`.
3. Run only one client at a time. Existing network, video, audio and input
   preferences and Windows-user-protected TURN credentials remain usable.
4. Sign in again, including 2FA if enabled. The old web-core login is not imported.
   Remembered native sessions use Windows CurrentUser protection.

Neither package contains `settings.json`; neither deletes old files or changes
host configuration. Old web-core files are no longer needed by EvoParseq.
Keep backups until you have verified the new client.

## Update window

**Help > Check for updates** checks public stable release metadata. It opens a
validated EvoParseq Windows ZIP in your browser; installation is manual, as in
ParsecWebTurn. Close the application before replacing its executable, keeping
`settings.json` beside it. No core-version check or background core download runs.
Only newer stable EvoParseq packages are offered. Development builds do not
receive older stable releases as updates.

## Publishing a transition (maintainers)

The native packaging script creates both the normal EvoParseq ZIP and the
legacy-named transition ZIP, with identical executable hashes and license notices.
Publish both on the repository's same **strictly newer stable release** for the
existing ParsecWebTurn updater to recognize the transition. For example, clients
on stable 0.9.4 require a stable version greater than 0.9.4. A development CI
artifact or prerelease is not offered by that stable updater.

The `dev` branch now develops EvoParseq. Version 0.9.5-dev.1 is a prerelease,
so the stable ParsecWebTurn updater does not offer it automatically. Download
the EvoParseq or transition package manually to try it. The `main` branch and
stable v0.9.4 remain ParsecWebTurn. A future stable EvoParseq release can use
the existing repository/update channel for the transition described above.
