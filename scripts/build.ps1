$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
Push-Location $taskRoot
try {
    cargo fmt --manifest-path src-native/Cargo.toml -- --check
    if ($LASTEXITCODE -ne 0) { throw 'Formatting failed' }
    cargo test --locked --manifest-path src-native/Cargo.toml -- --test-threads=1
    if ($LASTEXITCODE -ne 0) { throw 'EvoParseq tests failed' }
    cargo build --locked --release --manifest-path src-native/Cargo.toml
    if ($LASTEXITCODE -ne 0) { throw 'EvoParseq build failed' }
    & ./scripts/package-native.ps1
} finally { Pop-Location }
