param(
    [string]$Rom = "src/roms/dmg-acid2.gb",
    [string]$Out = "logs/gpu-structured.log",
    [int]$CaptureFirstN = 10,
    [int]$CaptureSkip = 8
)

$ErrorActionPreference = "Stop"
$env:GB_CAPTURE_FIRST_N = "$CaptureFirstN"
$env:GB_CAPTURE_PREFIX = "boot-frame"
$env:GB_CAPTURE_SKIP = "$CaptureSkip"
$env:GB_TRACE_STRUCTURED = "1"

# Keep logs emulator-only by redirecting just process output to target file.
cargo run --features trace_ppu --bin viewer -- $Rom *> $Out
Write-Host "Wrote $Out"
