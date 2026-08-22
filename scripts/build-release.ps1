Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$projectRoot = Split-Path -Parent $PSScriptRoot
$releaseRoot = Join-Path $projectRoot 'release'
$packageRoot = Join-Path $releaseRoot 'effigy-video-compressor'
$zipPath = Join-Path $releaseRoot 'effigy-compressor.zip'
$binaryPath = Join-Path $projectRoot 'build\cargo-target\release\effigy-video-compressor.exe'
$finishSoundPath = Join-Path $projectRoot 'finish.wav'

function Remove-ReleaseArtifact {
    param([string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        return
    }

    $resolved = (Resolve-Path -LiteralPath $Path).Path
    $expected = [System.IO.Path]::GetFullPath($Path)
    if ($resolved -ne $expected) {
        throw "Refusing to clean an unexpected release path: $resolved"
    }

    Remove-Item -LiteralPath $resolved -Recurse -Force
}

Push-Location $projectRoot
try {
    # Cargo's .cargo/config.toml directs all compilation output to build/cargo-target.
    & pnpm.cmd tauri build
    if ($LASTEXITCODE -ne 0) {
        throw "Tauri build failed with exit code $LASTEXITCODE."
    }

    if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) {
        throw "Expected executable was not produced: $binaryPath"
    }

    # Only generated distributables are replaced; source icons and build artifacts remain untouched.
    Remove-ReleaseArtifact $packageRoot
    Remove-ReleaseArtifact $zipPath
    New-Item -ItemType Directory -Force -Path $packageRoot | Out-Null

    Copy-Item -LiteralPath $binaryPath -Destination (Join-Path $packageRoot 'effigy-video-compressor.exe') -Force
    if (Test-Path -LiteralPath $finishSoundPath -PathType Leaf) {
        Copy-Item -LiteralPath $finishSoundPath -Destination (Join-Path $packageRoot 'finish.wav') -Force
    }

    # Keep the folder in the archive so extraction always produces a tidy portable app directory.
    Compress-Archive -LiteralPath $packageRoot -DestinationPath $zipPath -CompressionLevel Optimal -Force
}
finally {
    Pop-Location
}
