$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
Push-Location $taskRoot
try {
    $taskManifest = Get-Content -LiteralPath src-native/Cargo.toml -Raw
    $taskVersion = [regex]::Match($taskManifest, '(?m)^version = "([^"]+)"').Groups[1].Value
    if (!$taskVersion) { throw 'Missing EvoParseq version' }
    $taskSource = 'src-native/target/x86_64-pc-windows-msvc/release/evoparseq.exe'
    if (!(Test-Path -LiteralPath $taskSource)) { throw 'Build EvoParseq first' }
    $taskPackageName = "EvoParseq-v$taskVersion-win64"
    $taskPackage = Join-Path $taskRoot "dist/$taskPackageName"
    New-Item -ItemType Directory -Path $taskPackage -Force | Out-Null
    Copy-Item -LiteralPath $taskSource -Destination (Join-Path $taskPackage 'EvoParseq.exe')
    foreach ($taskFile in @('README.md','LICENSE','CHANGELOG.md')) {
        Copy-Item -LiteralPath $taskFile -Destination $taskPackage
    }
    Copy-Item -LiteralPath src-native/crates/bud/vendor/lowlat-client/LICENSE -Destination (Join-Path $taskPackage 'LOWLAT-LICENSE.txt')
    Copy-Item -LiteralPath src-native/crates/bud/vendor/lowlat-client/EVOPARSEQ.md -Destination (Join-Path $taskPackage 'BUD-LOCAL-CHANGES.md')
    Copy-Item -LiteralPath src-native/crates/bud/vendor/lowlat-net/EVOPARSEQ.md -Destination (Join-Path $taskPackage 'BUD-NET-LOCAL-CHANGES.md')
    Copy-Item -LiteralPath src-native/crates/video/LICENSE-lowlat -Destination (Join-Path $taskPackage 'GPU-LOWLAT-LICENSE.txt')
    git rev-parse HEAD | Set-Content -LiteralPath (Join-Path $taskPackage 'SOURCE.txt') -Encoding ascii
    Get-FileHash -Algorithm SHA256 (Join-Path $taskPackage 'EvoParseq.exe') | Format-List | Out-String | Set-Content -LiteralPath (Join-Path $taskPackage 'SHA256.txt')
    Compress-Archive -Path $taskPackage -DestinationPath (Join-Path $taskRoot "dist/$taskPackageName.zip") -Force
    Write-Output (Join-Path $taskRoot "dist/$taskPackageName.zip")
} finally { Pop-Location }
