$ErrorActionPreference = "Stop"

$scriptDir = $PSScriptRoot
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path

$package = "vertexlauncher"
$releaseDir = Join-Path $repoRoot "target/release"
$linuxGlibcVersion = if ($env:VERTEX_LINUX_GLIBC_VERSION) { $env:VERTEX_LINUX_GLIBC_VERSION } else { "2.17" }
$crossEnvVars = @("CFLAGS", "CXXFLAGS", "LDFLAGS", "CC", "CXX", "AR", "RANLIB", "RUSTFLAGS", "CARGO_BUILD_RUSTFLAGS")
$stagedArtifacts = @(
    "vertexlauncher-windowsx86-64.exe",
    "vertexlauncher-windowsarm64.exe",
    "vertexlauncher-linuxx86-64",
    "vertexlauncher-linuxarm64",
    "vertexlauncher-macosarm64",
    "vertexlauncher-windows-x86-64.exe",
    "vertexlauncher-windows-arm64.exe",
    "vertexlauncher-linux-arm64",
    "vertexlauncher-macos-aarch64",
    "vertexlauncher-windows-x86_64.exe",
    "vertexlauncher-linux-x86_64",
    "vertexlauncher-macos-x86_64"
)
$windowsTargets = @(
    @{ Target = "x86_64-pc-windows-msvc"; Platform = "windows"; Arch = "x86-64"; Extension = ".exe"; Builder = "xwin" },
    @{ Target = "aarch64-pc-windows-msvc"; Platform = "windows"; Arch = "arm64"; Extension = ".exe"; Builder = "xwin" }
)
$linuxTargets = @(
    @{ Target = "x86_64-unknown-linux-gnu"; Platform = "linux"; Arch = "x86-64"; Extension = ""; Builder = "zigbuild" },
    @{ Target = "aarch64-unknown-linux-gnu"; Platform = "linux"; Arch = "arm64"; Extension = ""; Builder = "zigbuild" }
)
$macosTargets = @(
    @{ Target = "aarch64-apple-darwin"; Platform = "macos"; Arch = "arm64"; Extension = ""; Builder = "zigbuild" }
)

function Require-CargoSubcommand {
    param(
        [Parameter(Mandatory = $true)][string]$Subcommand,
        [Parameter(Mandatory = $true)][string]$InstallHint
    )

    & cargo $Subcommand --help *> $null
    if ($LASTEXITCODE -ne 0) {
        throw "Missing cargo-$Subcommand. $InstallHint"
    }
}

function Get-StagedArtifactPath {
    param(
        [Parameter(Mandatory = $true)][string]$Platform,
        [Parameter(Mandatory = $true)][string]$Arch,
        [Parameter(Mandatory = $true)][string]$Extension
    )

    Join-Path $releaseDir "vertexlauncher-$Platform$Arch$Extension"
}

function Get-BuiltArtifactPath {
    param(
        [Parameter(Mandatory = $true)]$Spec,
        [Parameter(Mandatory = $true)][string]$Extension
    )

    $target = $Spec.Target
    if ($Spec.Target -eq "x86_64-unknown-linux-gnu") {
        $target = "$($Spec.Target).$linuxGlibcVersion"
    }

    Join-Path $repoRoot (Join-Path "target/$target/release" "$package$Extension")
}

function Clear-StagedArtifacts {
    foreach ($artifact in $stagedArtifacts) {
        $artifactPath = Join-Path $releaseDir $artifact
        if (Test-Path -LiteralPath $artifactPath -PathType Leaf) {
            Remove-Item -LiteralPath $artifactPath -Force
        }
    }
}

function Test-HasCrossPkgConfig {
    if ($env:PKG_CONFIG) {
        return $true
    }

    if ($env:PKG_CONFIG_ALLOW_CROSS -and ($env:PKG_CONFIG_SYSROOT_DIR -or $env:PKG_CONFIG_PATH -or $env:PKG_CONFIG_LIBDIR)) {
        return $true
    }

    return $false
}

function Test-HasMacOsSdk {
    return [bool](Resolve-MacOsSdkPath)
}

function Resolve-MacOsSdkPath {
    if ($env:SDKROOT -and (Test-Path -LiteralPath $env:SDKROOT -PathType Container)) {
        return $env:SDKROOT
    }

    if ($env:DEVELOPER_DIR -and (Test-Path -LiteralPath $env:DEVELOPER_DIR -PathType Container)) {
        $developerSdk = Join-Path $env:DEVELOPER_DIR "Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk"
        if (Test-Path -LiteralPath $developerSdk -PathType Container) {
            return $developerSdk
        }
    }

    $xcrun = Get-Command xcrun -ErrorAction SilentlyContinue
    if ($xcrun) {
        $sdkPath = & $xcrun.Source --sdk macosx --show-sdk-path 2>$null
        if ($LASTEXITCODE -eq 0 -and $sdkPath) {
            return $sdkPath.Trim()
        }
    }

    $sdkCandidates = Get-ChildItem -Path (Join-Path $HOME ".local/share/macos-sdk") -Filter "MacOSX*.sdk" -Directory -ErrorAction SilentlyContinue
    if ($sdkCandidates) {
        return $sdkCandidates[0].FullName
    }

    return $null
}

function Invoke-BuildCommand {
    param(
        [Parameter(Mandatory = $true)]$Spec
    )

    switch ($Spec.Builder) {
        "cargo" {
            & cargo build --release --target $Spec.Target -p $package
        }
        "zigbuild" {
            $savedSdkRoot = $env:SDKROOT
            $buildTarget = $Spec.Target
            if ($Spec.Target -eq "x86_64-unknown-linux-gnu") {
                $buildTarget = "$($Spec.Target).$linuxGlibcVersion"
            }
            if ($Spec.Target -eq "aarch64-apple-darwin") {
                $sdkRoot = Resolve-MacOsSdkPath
                if ($sdkRoot) {
                    $env:SDKROOT = $sdkRoot
                }
            }

            try {
                & cargo zigbuild --release --target $buildTarget -p $package
            }
            finally {
                if ($null -eq $savedSdkRoot) {
                    Remove-Item "Env:SDKROOT" -ErrorAction SilentlyContinue
                }
                else {
                    Set-Item "Env:SDKROOT" $savedSdkRoot
                }
            }
        }
        "xwin" {
            $savedEnv = @{}
            foreach ($varName in $crossEnvVars) {
                if (Test-Path "Env:$varName") {
                    $savedEnv[$varName] = (Get-Item "Env:$varName").Value
                    Remove-Item "Env:$varName" -ErrorAction SilentlyContinue
                }
                else {
                    $savedEnv[$varName] = $null
                }
            }

            try {
                & cargo xwin build --release --target $Spec.Target --cross-compiler clang -p $package
            }
            finally {
                foreach ($varName in $crossEnvVars) {
                    if ($null -eq $savedEnv[$varName]) {
                        Remove-Item "Env:$varName" -ErrorAction SilentlyContinue
                    }
                    else {
                        Set-Item "Env:$varName" $savedEnv[$varName]
                    }
                }
            }
        }
        default {
            throw "Unsupported builder '$($Spec.Builder)'"
        }
    }

    if ($LASTEXITCODE -ne 0) {
        throw "cargo $($Spec.Builder) build --release --target $($Spec.Target) -p $package failed with exit code $LASTEXITCODE"
    }
}

function Build-And-StageArtifact {
    param(
        [Parameter(Mandatory = $true)]$Spec
    )

    Write-Host "Building $($Spec.Platform) $($Spec.Arch) release binary..."

    if ($Spec.Target -eq "aarch64-unknown-linux-gnu" -and -not (Test-HasCrossPkgConfig)) {
        throw "Linux arm64 requires PKG_CONFIG_ALLOW_CROSS=1 plus either PKG_CONFIG=<wrapper> or PKG_CONFIG_SYSROOT_DIR with PKG_CONFIG_PATH/PKG_CONFIG_LIBDIR."
    }

    if ($Spec.Target -eq "aarch64-apple-darwin" -and -not (Test-HasMacOsSdk)) {
        throw "macOS arm64 requires an Apple SDK via SDKROOT, DEVELOPER_DIR, xcrun, or ~/.local/share/macos-sdk/MacOSX*.sdk."
    }

    Invoke-BuildCommand -Spec $Spec

    $builtArtifact = Get-BuiltArtifactPath -Spec $Spec -Extension $Spec.Extension
    $stagedArtifact = Get-StagedArtifactPath -Platform $Spec.Platform -Arch $Spec.Arch -Extension $Spec.Extension

    if (-not (Test-Path -LiteralPath $builtArtifact -PathType Leaf)) {
        throw "Missing built artifact: $builtArtifact"
    }

    Copy-Item -LiteralPath $builtArtifact -Destination $stagedArtifact -Force
    Write-Host "  Staged: $stagedArtifact"
}

Push-Location $repoRoot
try {
    New-Item -ItemType Directory -Force -Path $releaseDir | Out-Null
    Clear-StagedArtifacts

    Require-CargoSubcommand -Subcommand "xwin" -InstallHint "Install it with: cargo install --locked cargo-xwin"
    Require-CargoSubcommand -Subcommand "zigbuild" -InstallHint "Install it with: cargo install --locked cargo-zigbuild"

    $failures = @()
    foreach ($spec in $windowsTargets) {
        try {
            Build-And-StageArtifact -Spec $spec
        }
        catch {
            $failures += "$($spec.Platform) $($spec.Arch): $($_.Exception.Message)"
        }
    }
    foreach ($spec in $linuxTargets) {
        try {
            Build-And-StageArtifact -Spec $spec
        }
        catch {
            $failures += "$($spec.Platform) $($spec.Arch): $($_.Exception.Message)"
        }
    }
    foreach ($spec in $macosTargets) {
        try {
            Build-And-StageArtifact -Spec $spec
        }
        catch {
            $failures += "$($spec.Platform) $($spec.Arch): $($_.Exception.Message)"
        }
    }

    Write-Host ""
    Write-Host "Artifacts ready:"
    foreach ($spec in ($windowsTargets + $linuxTargets + $macosTargets)) {
        Write-Host "  $(Get-StagedArtifactPath -Platform $spec.Platform -Arch $spec.Arch -Extension $spec.Extension)"
    }

    if ($failures.Count -gt 0) {
        Write-Error ("Build matrix incomplete:`n  - " + ($failures -join "`n  - "))
    }
}
finally {
    Pop-Location
}
