# MmdSkin 1.21.1 / NeoForge ragdoll preview build and package check.
# Run from PowerShell on Windows x64, with JDK 21, Rust, and MSVC installed.
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-LastExitCode([string]$step) {
    if ($LASTEXITCODE -ne 0) {
        throw "$step failed (exit code: $LASTEXITCODE)."
    }
}

function Require-Command([string]$commandName) {
    if (-not (Get-Command $commandName -ErrorAction SilentlyContinue)) {
        throw "Required command not found: $commandName"
    }
}

$repoRoot = $PSScriptRoot
$dllRelative = 'rust_engine/target/release/mmd_engine.dll'
$dllJarEntry = 'natives/windows-x64/mmd_engine.dll'
$mixinJarEntry = 'com/shiroha/mmdskin/mixin/neoforge/compat/PhysicsModDeathMixin.class'

Push-Location -LiteralPath $repoRoot
try {
    if (-not [Environment]::Is64BitOperatingSystem -or [Environment]::OSVersion.Platform -ne 'Win32NT') {
        throw 'This build script requires Windows x64. For other platforms, build manually.'
    }

    Require-Command 'javac'
    Require-Command 'cargo'
    Require-Command 'jar'

    $javacVersion = (& javac -version 2>&1 | Out-String).Trim()
    Assert-LastExitCode 'Checking Java'
    if ($javacVersion -notmatch '^javac\s+21(?:\.|$)') {
        throw "JDK 21 is required; detected: $javacVersion"
    }

    if (-not (Test-Path -LiteralPath 'rust_engine/deps/bullet3/src/BulletDynamics')) {
        throw 'Bullet3 sources are missing. Run: git submodule update --init --recursive'
    }

    Write-Host "[1/3] Compiling Windows native engine with $javacVersion"
    & cargo build --release --manifest-path rust_engine/Cargo.toml
    Assert-LastExitCode 'Rust native compilation'
    if (-not (Test-Path -LiteralPath $dllRelative)) {
        throw "Rust completed without producing $dllRelative"
    }

    # Gradle copyRustNatives collects the newly compiled DLL from target/release.
    Write-Host '[2/3] Compiling NeoForge mod with bundled native engine'
    & .\gradlew.bat :neoforge:remapJar --no-daemon -x test
    Assert-LastExitCode 'NeoForge JAR compilation'

    Write-Host '[3/3] Verifying JAR classes and native library'
    $outputs = @(Get-ChildItem -LiteralPath 'neoforge/build/libs' -Filter 'mmdskin-neoforge-*.jar' -File |
        Where-Object { $_.Name -notmatch '-dev|-sources' } |
        Sort-Object -Property LastWriteTimeUtc -Descending)
    if ($outputs.Count -eq 0) {
        throw 'No NeoForge release JAR was found in neoforge/build/libs.'
    }

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $nativeHash = (Get-FileHash -LiteralPath $dllRelative -Algorithm SHA256).Hash
    $successful = @()
    foreach ($output in $outputs) {
        $archive = [System.IO.Compression.ZipFile]::OpenRead($output.FullName)
        try {
            $native = $archive.GetEntry($dllJarEntry)
            $mixin = $archive.GetEntry($mixinJarEntry)
            if ($null -eq $native -or $null -eq $mixin) {
                continue
            }

            $stream = $native.Open()
            $hasher = [System.Security.Cryptography.SHA256]::Create()
            try {
                $embeddedHash = [BitConverter]::ToString($hasher.ComputeHash($stream)).Replace('-', '')
            }
            finally {
                $stream.Dispose()
                $hasher.Dispose()
            }

            if ($embeddedHash -eq $nativeHash) {
                $successful += $output.FullName
            }
        }
        finally {
            $archive.Dispose()
        }
    }

    if ($successful.Count -eq 0) {
        throw 'JAR verification failed: missing Physics Mod Mixin or mismatched Windows DLL.'
    }

    Write-Host 'Windows NeoForge ragdoll preview build is ready:' -ForegroundColor Green
    foreach ($path in $successful) {
        Write-Host "  $path" -ForegroundColor Green
    }
    Write-Host 'This is a build check only; in-game ragdoll behaviour still needs testing.'
}
finally {
    Pop-Location
}
