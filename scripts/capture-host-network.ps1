param(
    [Parameter(Mandatory)][string]$OutputDirectory,
    [ValidateRange(10,120)][int]$Seconds = 60
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$log = Join-Path $OutputDirectory 'capture-status.txt'
$started = $false
$filters = @()
function Invoke-Pktmon {
    param([string[]]$Arguments)
    $output = & pktmon @Arguments 2>&1
    $code = $LASTEXITCODE
    $output | Out-File -Append -Encoding utf8 -FilePath $log
    if ($code -ne 0) { throw "pktmon failed ($code): $($Arguments -join ' ')" }
    return ($output -join "`n")
}
try {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'Run this isolated packet capture as administrator.'
    }
    $status = Invoke-Pktmon -Arguments @('status')
    if ($status -notmatch '(?i)\bnot running\b') {
        throw 'Packet Monitor is active or its status could not be recognized; leave it untouched.'
    }
    $existing = Invoke-Pktmon -Arguments @('filter','list')
    if ($existing -notmatch '(?im)^\s*None\s*$|no filters|no packet filters|nincs') {
        throw 'Existing or unrecognized Packet Monitor filters: leave them untouched and stop.'
    }
    $ids = @(Get-Process parsecd -ErrorAction Stop).Id
    $ports = @(Get-NetUDPEndpoint | Where-Object OwningProcess -in $ids |
        Select-Object -ExpandProperty LocalPort -Unique)
    if ($ports.Count -eq 0) { throw 'No active Parsec UDP socket found.' }
    foreach ($port in $ports) {
        $name = "PWT-$PID-$port"
        $null = Invoke-Pktmon -Arguments @('filter','add',$name,'-t','UDP','-p',"$port")
        $filters += $name
    }
    $etl = Join-Path $OutputDirectory 'host-network.etl'
    $null = Invoke-Pktmon -Arguments @('start','--capture','--comp','nics','--pkt-size','96',
        '--file-size','64','--file-name',$etl)
    $started = $true
    $begin = [DateTimeOffset]::UtcNow
    Start-Sleep -Seconds $Seconds
    $null = Invoke-Pktmon -Arguments @('stop')
    $started = $false
    @{schema=1;started_utc=$begin.ToString('o');finished_utc=[DateTimeOffset]::UtcNow.ToString('o');
      ports=$ports;capture_bytes_per_packet=96;scope='encrypted-parsec-udp-at-nic';
      audio_identifiable=$false} | ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $OutputDirectory 'capture.json')
    $null = Invoke-Pktmon -Arguments @('etl2pcap',$etl,'--out',(Join-Path $OutputDirectory 'host-network.pcapng'))
    $null = Invoke-Pktmon -Arguments @('etl2txt',$etl,'--out',(Join-Path $OutputDirectory 'host-network.txt'),'--brief')
    'COMPLETE' | Out-File -Append -Encoding utf8 -FilePath $log
} catch {
    $_.Exception.Message | Out-File -Append -Encoding utf8 -FilePath $log
    exit 1
} finally {
    if ($started) { & pktmon stop 2>&1 | Out-File -Append -Encoding utf8 -FilePath $log }
    if ($filters.Count -gt 0) {
        # Pktmon removes ALL filters, not one named filter. Remove only if the
        # whole current list is ours; leave concurrent additions untouched.
        $current = & pktmon filter list 2>&1
        $filterExit = $LASTEXITCODE
        $names = @($current | ForEach-Object {
            if ($_ -match '^\s*\d+\s+(\S+)') { $Matches[1] }
        })
        if ($filterExit -eq 0 -and $names.Count -gt 0 -and
            @($names | Where-Object { $_ -notin $filters }).Count -eq 0) {
            & pktmon filter remove 2>&1 | Out-File -Append -Encoding utf8 -FilePath $log
        } else {
            'Filter list changed or could not be verified; left untouched.' |
                Out-File -Append -Encoding utf8 -FilePath $log
        }
    }
}
