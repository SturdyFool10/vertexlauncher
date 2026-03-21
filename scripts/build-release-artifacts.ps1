$TargetFilters = @()
$remainingArgs = @()
foreach ($arg in $args) {
    if ($arg -and -not $arg.StartsWith("-")) {
        $TargetFilters += $arg.Trim()
    }
    else {
        $remainingArgs += $arg
    }
}
if ($remainingArgs.Count -gt 0) {
    throw "Unsupported arguments: $($remainingArgs -join ', ')"
}

$ErrorActionPreference = "Stop"

$scriptDir = $PSScriptRoot
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path

$package = "vertexlauncher"
$releaseDir = Join-Path $repoRoot "target/release"
$flatpakAppId = "io.github.SturdyFool10.VertexLauncher"
$flatpakBranch = if ($env:VERTEX_RELEASE_FLATPAK_BRANCH) { $env:VERTEX_RELEASE_FLATPAK_BRANCH } elseif ($env:VERTEX_FLATPAK_BRANCH) { $env:VERTEX_FLATPAK_BRANCH } else { "stable" }
$flatpakArtifactArches = @()
$appImageArtifactArches = @()
$crossEnvVars = @("CFLAGS", "CXXFLAGS", "LDFLAGS", "CC", "CXX", "AR", "RANLIB", "RUSTFLAGS", "CARGO_BUILD_RUSTFLAGS")
$stagedArtifacts = @(
    "vertexlauncher-windowsx86-64.exe",
    "vertexlauncher-windowsarm64.exe",
    "vertexlauncher-linuxx86-64",
    "vertexlauncher-linuxarm64",
    "vertexlauncher-linuxx86-64.AppImage",
    "vertexlauncher-linuxarm64.AppImage",
    "vertexlauncher-macosarm64",
    "vertexlauncher-windows-x86-64.exe",
    "vertexlauncher-windows-arm64.exe",
    "vertexlauncher-linux-arm64",
    "vertexlauncher-linux-arm64.AppImage",
    "vertexlauncher-macos-aarch64",
    "vertexlauncher-windows-x86_64.exe",
    "vertexlauncher-linux-x86_64",
    "vertexlauncher-linux-x86_64.AppImage",
    "vertexlauncher-macos-x86_64",
    "$flatpakAppId-x86_64.flatpak",
    "$flatpakAppId-aarch64.flatpak"
)
$windowsTargets = @(
    @{ Target = "x86_64-pc-windows-msvc"; Platform = "windows"; Arch = "x86-64"; Extension = ".exe"; Builder = "xwin" },
    @{ Target = "aarch64-pc-windows-msvc"; Platform = "windows"; Arch = "arm64"; Extension = ".exe"; Builder = "xwin" }
)
$linuxTargets = @(
    @{ Target = "x86_64-unknown-linux-gnu"; Platform = "linux"; Arch = "x86-64"; Extension = ""; Builder = "linux-container" },
    @{ Target = "aarch64-unknown-linux-gnu"; Platform = "linux"; Arch = "arm64"; Extension = ""; Builder = "linux-arm64-container" }
)
$macosTargets = @(
    @{ Target = "aarch64-apple-darwin"; Platform = "macos"; Arch = "arm64"; Extension = ""; Builder = "zigbuild" }
)

function Resolve-RequestedTargets {
    $rawTargets = @()

    if ($TargetFilters.Count -gt 0) {
        $rawTargets += $TargetFilters
    }

    if ($env:VERTEX_RELEASE_TARGETS) {
        $rawTargets += $env:VERTEX_RELEASE_TARGETS.Split(",", [System.StringSplitOptions]::RemoveEmptyEntries)
    }

    $requestedTargets = @()
    foreach ($target in $rawTargets) {
        $trimmed = $target.Trim()
        if ($trimmed -and $requestedTargets -notcontains $trimmed) {
            $requestedTargets += $trimmed
        }
    }

    return $requestedTargets
}

function Validate-RequestedTargets {
    param(
        [Parameter(Mandatory = $true)]$AllSpecs,
        [Parameter(Mandatory = $true)][string[]]$RequestedTargets
    )

    if ($RequestedTargets.Count -eq 0) {
        return
    }

    $knownTargets = @($AllSpecs | ForEach-Object { $_.Target })
    $unknownTargets = @($RequestedTargets | Where-Object { $knownTargets -notcontains $_ })
    if ($unknownTargets.Count -gt 0) {
        throw "Unsupported target filter(s): $($unknownTargets -join ', ')"
    }
}

function Filter-TargetSpecs {
    param(
        [Parameter(Mandatory = $true)]$Specs,
        [Parameter(Mandatory = $true)][string[]]$RequestedTargets
    )

    if ($RequestedTargets.Count -eq 0) {
        return @($Specs)
    }

    return @($Specs | Where-Object { $RequestedTargets -contains $_.Target })
}

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

    Join-Path $repoRoot (Join-Path "target/$($Spec.Target)/release" "$package$Extension")
}

function Get-FlatpakArtifactPath {
    param(
        [Parameter(Mandatory = $true)][string]$Arch
    )

    Join-Path $releaseDir "$flatpakAppId-$Arch.flatpak"
}

function Get-AppImageArtifactPath {
    param(
        [Parameter(Mandatory = $true)][string]$Arch
    )

    Join-Path $releaseDir "vertexlauncher-linux$Arch.AppImage"
}

function Normalize-PackagingArch {
    param(
        [Parameter(Mandatory = $true)][string]$Arch
    )

    $trimmed = $Arch.Trim()
    switch ($trimmed) {
        { $_ -in @("x86_64", "amd64", "x86-64") } { return "x86_64" }
        { $_ -in @("aarch64", "arm64") } { return "aarch64" }
        default { throw "Unsupported Linux architecture '$Arch'." }
    }
}

function Get-DefaultReleaseLinuxArches {
    if (-not $IsLinux) {
        return @()
    }

    $machine = (& uname -m).Trim()
    switch ($machine) {
        { $_ -in @("x86_64", "amd64") } { return @("x86_64", "aarch64") }
        { $_ -in @("aarch64", "arm64") } { return @("aarch64") }
        default { return @() }
    }
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
        "linux-container" {
            $helperScript = Join-Path $repoRoot "scripts/build-linux-x86_64-container.sh"
            if (-not (Test-Path -LiteralPath $helperScript -PathType Leaf)) {
                throw "Missing Linux x86-64 container helper: $helperScript"
            }

            $bash = Get-Command bash -ErrorAction SilentlyContinue
            if (-not $bash) {
                throw "Linux x86-64 containerized release builds require bash on PATH."
            }

            $podman = Get-Command podman -ErrorAction SilentlyContinue
            if (-not $podman) {
                throw "Linux x86-64 containerized release builds require podman on PATH."
            }

            & $bash.Source $helperScript
        }
        "linux-arm64-container" {
            $helperScript = Join-Path $repoRoot "scripts/build-linux-arm64-container.sh"
            if (-not (Test-Path -LiteralPath $helperScript -PathType Leaf)) {
                throw "Missing Linux arm64 container helper: $helperScript"
            }

            $bash = Get-Command bash -ErrorAction SilentlyContinue
            if (-not $bash) {
                throw "Linux arm64 containerized release builds require bash on PATH."
            }

            $podman = Get-Command podman -ErrorAction SilentlyContinue
            if (-not $podman) {
                throw "Linux arm64 containerized release builds require podman on PATH."
            }

            & $bash.Source $helperScript
        }
        "zigbuild" {
            $savedSdkRoot = $env:SDKROOT
            if ($Spec.Target -eq "aarch64-apple-darwin") {
                $sdkRoot = Resolve-MacOsSdkPath
                if ($sdkRoot) {
                    $env:SDKROOT = $sdkRoot
                }
            }

            try {
                & cargo zigbuild --release --target $Spec.Target -p $package
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

    if ($Spec.Target -eq "aarch64-unknown-linux-gnu" -and $Spec.Builder -eq "zigbuild" -and -not (Test-HasCrossPkgConfig)) {
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

function Build-FlatpakArtifacts {
    $helperScript = Join-Path $repoRoot "scripts/build-flatpak.sh"
    if (-not (Test-Path -LiteralPath $helperScript -PathType Leaf)) {
        throw "Missing Flatpak helper: $helperScript"
    }

    $bash = Get-Command bash -ErrorAction SilentlyContinue
    if (-not $bash) {
        throw "Flatpak builds require bash on PATH."
    }

    if (-not (Get-Command flatpak -ErrorAction SilentlyContinue)) {
        throw "Flatpak builds require flatpak on PATH."
    }

    $rawRequestedArches = if ($env:VERTEX_RELEASE_FLATPAK_ARCHES) {
        $env:VERTEX_RELEASE_FLATPAK_ARCHES.Split(",", [System.StringSplitOptions]::RemoveEmptyEntries)
    }
    elseif ($env:VERTEX_FLATPAK_ARCHES) {
        $env:VERTEX_FLATPAK_ARCHES.Split(",", [System.StringSplitOptions]::RemoveEmptyEntries)
    }
    else {
        $defaults = Get-DefaultReleaseLinuxArches
        if ($defaults.Count -gt 0) {
            $defaults
        }
        else {
            @((& flatpak --default-arch).Trim())
        }
    }

    $requestedArches = @()
    foreach ($arch in $rawRequestedArches) {
        if (-not $arch) {
            continue
        }

        $normalizedArch = Normalize-PackagingArch -Arch $arch
        if ($requestedArches -notcontains $normalizedArch) {
            $requestedArches += $normalizedArch
        }
    }

    $requestedArchList = $requestedArches -join ","
    $savedFlatpakBranch = $env:VERTEX_FLATPAK_BRANCH
    $savedFlatpakArches = $env:VERTEX_FLATPAK_ARCHES
    $savedFlatpakEmulation = $env:VERTEX_ENABLE_ARM64_EMULATION
    Write-Host "Building Flatpak release bundle..."

    try {
        $env:VERTEX_FLATPAK_BRANCH = $flatpakBranch
        $env:VERTEX_FLATPAK_ARCHES = $requestedArchList
        if ($requestedArches -contains "aarch64") {
            $env:VERTEX_ENABLE_ARM64_EMULATION = "1"
        }
        else {
            Remove-Item "Env:VERTEX_ENABLE_ARM64_EMULATION" -ErrorAction SilentlyContinue
        }
        & $bash.Source $helperScript
        if ($LASTEXITCODE -ne 0) {
            throw "Flatpak helper failed with exit code $LASTEXITCODE"
        }
    }
    finally {
        if ($null -eq $savedFlatpakBranch) {
            Remove-Item "Env:VERTEX_FLATPAK_BRANCH" -ErrorAction SilentlyContinue
        }
        else {
            Set-Item "Env:VERTEX_FLATPAK_BRANCH" $savedFlatpakBranch
        }

        if ($null -eq $savedFlatpakArches) {
            Remove-Item "Env:VERTEX_FLATPAK_ARCHES" -ErrorAction SilentlyContinue
        }
        else {
            Set-Item "Env:VERTEX_FLATPAK_ARCHES" $savedFlatpakArches
        }

        if ($null -eq $savedFlatpakEmulation) {
            Remove-Item "Env:VERTEX_ENABLE_ARM64_EMULATION" -ErrorAction SilentlyContinue
        }
        else {
            Set-Item "Env:VERTEX_ENABLE_ARM64_EMULATION" $savedFlatpakEmulation
        }
    }

    $script:flatpakArtifactArches = @()
    foreach ($arch in $requestedArches) {
        $normalizedArch = $arch.Trim()
        if (-not $normalizedArch) {
            continue
        }

        $artifactPath = Get-FlatpakArtifactPath -Arch $normalizedArch
        if (-not (Test-Path -LiteralPath $artifactPath -PathType Leaf)) {
            throw "Missing built Flatpak artifact: $artifactPath"
        }

        $script:flatpakArtifactArches += $normalizedArch
        Write-Host "  Staged: $artifactPath"
    }
}

function Get-CurrentLinuxAppImageArch {
    if (-not $IsLinux) {
        return $null
    }

    $machine = (& uname -m).Trim()
    switch ($machine) {
        { $_ -in @("x86_64", "amd64") } { return "x86_64" }
        { $_ -in @("aarch64", "arm64") } { return "aarch64" }
        default { return $null }
    }
}

function Build-AppImageArtifacts {
    if (-not $IsLinux) {
        throw "AppImage builds require a native Linux host."
    }

    $helperScript = Join-Path $repoRoot "scripts/build-appimage.sh"
    if (-not (Test-Path -LiteralPath $helperScript -PathType Leaf)) {
        throw "Missing AppImage helper: $helperScript"
    }

    $bash = Get-Command bash -ErrorAction SilentlyContinue
    if (-not $bash) {
        throw "AppImage builds require bash on PATH."
    }

    $rawRequestedArches = if ($env:VERTEX_RELEASE_APPIMAGE_ARCHES) {
        $env:VERTEX_RELEASE_APPIMAGE_ARCHES.Split(",", [System.StringSplitOptions]::RemoveEmptyEntries)
    }
    elseif ($env:VERTEX_APPIMAGE_ARCHES) {
        $env:VERTEX_APPIMAGE_ARCHES.Split(",", [System.StringSplitOptions]::RemoveEmptyEntries)
    }
    elseif ($env:VERTEX_RELEASE_APPIMAGE_ARCH) {
        @($env:VERTEX_RELEASE_APPIMAGE_ARCH.Trim())
    }
    elseif ($env:VERTEX_APPIMAGE_ARCH) {
        @($env:VERTEX_APPIMAGE_ARCH.Trim())
    }
    else {
        $defaults = Get-DefaultReleaseLinuxArches
        if ($defaults.Count -gt 0) {
            $defaults
        }
        else {
            @((Get-CurrentLinuxAppImageArch))
        }
    }

    $requestedArches = @()
    foreach ($arch in $rawRequestedArches) {
        if (-not $arch) {
            continue
        }

        $normalizedArch = Normalize-PackagingArch -Arch $arch
        if ($requestedArches -notcontains $normalizedArch) {
            $requestedArches += $normalizedArch
        }
    }

    if ($requestedArches.Count -eq 0) {
        throw "Unsupported AppImage host architecture."
    }

    $script:appImageArtifactArches = @()
    foreach ($requestedArch in $requestedArches) {
        switch ($requestedArch) {
            "x86_64" {
                $target = "x86_64-unknown-linux-gnu"
                $stagedArch = "x86-64"
            }
            "aarch64" {
                $target = "aarch64-unknown-linux-gnu"
                $stagedArch = "arm64"
            }
        }

        $savedAppImageArch = $env:VERTEX_APPIMAGE_ARCH
        $savedAppImageTarget = $env:VERTEX_APPIMAGE_TARGET
        $savedAppImageSource = $env:VERTEX_APPIMAGE_SOURCE
        $savedAppImageEmulation = $env:VERTEX_ENABLE_ARM64_EMULATION
        Write-Host "Building AppImage release bundle for $stagedArch..."

        try {
            $env:VERTEX_APPIMAGE_ARCH = $requestedArch
            $env:VERTEX_APPIMAGE_TARGET = $target
            $env:VERTEX_APPIMAGE_SOURCE = (Join-Path $repoRoot "target/$target/release/$package")
            if ($requestedArch -eq "aarch64") {
                $env:VERTEX_ENABLE_ARM64_EMULATION = "1"
            }
            else {
                Remove-Item "Env:VERTEX_ENABLE_ARM64_EMULATION" -ErrorAction SilentlyContinue
            }

            & $bash.Source $helperScript
            if ($LASTEXITCODE -ne 0) {
                throw "AppImage helper failed with exit code $LASTEXITCODE"
            }
        }
        finally {
            if ($null -eq $savedAppImageArch) {
                Remove-Item "Env:VERTEX_APPIMAGE_ARCH" -ErrorAction SilentlyContinue
            }
            else {
                Set-Item "Env:VERTEX_APPIMAGE_ARCH" $savedAppImageArch
            }

            if ($null -eq $savedAppImageTarget) {
                Remove-Item "Env:VERTEX_APPIMAGE_TARGET" -ErrorAction SilentlyContinue
            }
            else {
                Set-Item "Env:VERTEX_APPIMAGE_TARGET" $savedAppImageTarget
            }

            if ($null -eq $savedAppImageSource) {
                Remove-Item "Env:VERTEX_APPIMAGE_SOURCE" -ErrorAction SilentlyContinue
            }
            else {
                Set-Item "Env:VERTEX_APPIMAGE_SOURCE" $savedAppImageSource
            }

            if ($null -eq $savedAppImageEmulation) {
                Remove-Item "Env:VERTEX_ENABLE_ARM64_EMULATION" -ErrorAction SilentlyContinue
            }
            else {
                Set-Item "Env:VERTEX_ENABLE_ARM64_EMULATION" $savedAppImageEmulation
            }
        }

        $artifactPath = Get-AppImageArtifactPath -Arch $stagedArch
        if (-not (Test-Path -LiteralPath $artifactPath -PathType Leaf)) {
            throw "Missing built AppImage artifact: $artifactPath"
        }

        $script:appImageArtifactArches += $stagedArch
        Write-Host "  Staged: $artifactPath"
    }
}

Push-Location $repoRoot
try {
    $requestedTargets = Resolve-RequestedTargets
    $allTargets = @($windowsTargets + $linuxTargets + $macosTargets)
    Validate-RequestedTargets -AllSpecs $allTargets -RequestedTargets $requestedTargets
    $selectedWindowsTargets = Filter-TargetSpecs -Specs $windowsTargets -RequestedTargets $requestedTargets
    $selectedLinuxTargets = Filter-TargetSpecs -Specs $linuxTargets -RequestedTargets $requestedTargets
    $selectedMacosTargets = Filter-TargetSpecs -Specs $macosTargets -RequestedTargets $requestedTargets

    New-Item -ItemType Directory -Force -Path $releaseDir | Out-Null
    Clear-StagedArtifacts

    Require-CargoSubcommand -Subcommand "xwin" -InstallHint "Install it with: cargo install --locked cargo-xwin"
    Require-CargoSubcommand -Subcommand "zigbuild" -InstallHint "Install it with: cargo install --locked cargo-zigbuild"

    $failures = @()
    foreach ($spec in $selectedWindowsTargets) {
        try {
            Build-And-StageArtifact -Spec $spec
        }
        catch {
            $failures += "$($spec.Platform) $($spec.Arch): $($_.Exception.Message)"
        }
    }
    foreach ($spec in $selectedLinuxTargets) {
        try {
            Build-And-StageArtifact -Spec $spec
        }
        catch {
            $failures += "$($spec.Platform) $($spec.Arch): $($_.Exception.Message)"
        }
    }
    foreach ($spec in $selectedMacosTargets) {
        try {
            Build-And-StageArtifact -Spec $spec
        }
        catch {
            $failures += "$($spec.Platform) $($spec.Arch): $($_.Exception.Message)"
        }
    }

    if ($requestedTargets.Count -eq 0) {
        try {
            Build-FlatpakArtifacts
        }
        catch {
            $failures += "flatpak: $($_.Exception.Message)"
        }

        try {
            Build-AppImageArtifacts
        }
        catch {
            $failures += "appimage: $($_.Exception.Message)"
        }
    }

    Write-Host ""
    Write-Host "Artifacts ready:"
    foreach ($spec in ($selectedWindowsTargets + $selectedLinuxTargets + $selectedMacosTargets)) {
        Write-Host "  $(Get-StagedArtifactPath -Platform $spec.Platform -Arch $spec.Arch -Extension $spec.Extension)"
    }
    if ($requestedTargets.Count -eq 0) {
        foreach ($arch in $flatpakArtifactArches) {
            Write-Host "  $(Get-FlatpakArtifactPath -Arch $arch)"
        }
        foreach ($arch in $appImageArtifactArches) {
            Write-Host "  $(Get-AppImageArtifactPath -Arch $arch)"
        }
    }

    if ($failures.Count -gt 0) {
        Write-Error ("Build matrix incomplete:`n  - " + ($failures -join "`n  - "))
    }
}
finally {
    Pop-Location
}
