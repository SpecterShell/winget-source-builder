[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$WingetCliRoot,

    [string]$Configuration = "Debug",

    [string]$Platform = "x64",

    [string]$Destination
)

$ErrorActionPreference = "Stop"

function Normalize-PathString {
    param([Parameter(Mandatory = $true)][string]$Path)

    $normalized = $Path
    $providerPrefix = "Microsoft.PowerShell.Core\FileSystem::"
    if ($normalized.StartsWith($providerPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        $normalized = $normalized.Substring($providerPrefix.Length)
    }

    if ($normalized.StartsWith("\\?\", [System.StringComparison]::Ordinal)) {
        $normalized = $normalized.Substring(4)
    }

    return $normalized
}

function Resolve-RequiredPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    $resolved = Resolve-Path -LiteralPath (Normalize-PathString $Path) -ErrorAction Stop
    return Normalize-PathString $resolved.Path
}

function Get-VcpkgTriplet {
    param(
        [Parameter(Mandatory = $true)][string]$Platform,
        [Parameter(Mandatory = $true)][string]$Configuration
    )

    $platformTriplet = switch ($Platform.ToLowerInvariant()) {
        "win32" { "x86" }
        default { $Platform.ToLowerInvariant() }
    }

    $triplet = switch ($Configuration.ToLowerInvariant()) {
        "debug" { $platformTriplet }
        "release" { "$platformTriplet-release" }
        "releasestatic" { "$platformTriplet-release-static" }
        "fuzzing" { "$platformTriplet-fuzzing" }
        default { "$platformTriplet-$($Configuration.ToLowerInvariant())" }
    }

    return $triplet
}

function Get-HostPlatform {
    if ($env:PROCESSOR_ARCHITECTURE) {
        switch ($env:PROCESSOR_ARCHITECTURE.ToUpperInvariant()) {
            "AMD64" { return "x64" }
            "ARM64" { return "arm64" }
            "X86" { return "x86" }
        }
    }

    return "x64"
}

$wingetCliRoot = Resolve-RequiredPath $WingetCliRoot
$solutionPath = Join-Path $wingetCliRoot "src\AppInstallerCLI.sln"
if (-not (Test-Path -LiteralPath $solutionPath)) {
    throw "winget-cli solution was not found at $solutionPath"
}

$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path -LiteralPath $vswhere)) {
    throw "vswhere.exe was not found at $vswhere"
}

$visualStudioPath = & $vswhere -latest -products * -property installationPath
if (-not $visualStudioPath) {
    throw "Visual Studio installation was not found."
}

$visualStudioPath = $visualStudioPath.Trim()
$vsDevCmd = Join-Path $visualStudioPath "Common7\Tools\VsDevCmd.bat"
$vcvarsall = Join-Path $visualStudioPath "VC\Auxiliary\Build\vcvarsall.bat"
$msbuild = Join-Path $visualStudioPath "MSBuild\Current\Bin\MSBuild.exe"

if (-not (Test-Path -LiteralPath $vsDevCmd) -and -not (Test-Path -LiteralPath $vcvarsall)) {
    throw "Neither VsDevCmd.bat nor vcvarsall.bat was found under $visualStudioPath"
}

if (-not (Test-Path -LiteralPath $msbuild)) {
    throw "MSBuild.exe was not found at $msbuild"
}

$hostPlatform = Get-HostPlatform
$vcvarsPlatform = if ($Platform.ToLowerInvariant() -eq $hostPlatform.ToLowerInvariant()) {
    $Platform
} else {
    "$hostPlatform`_$Platform"
}

$developerEnvironment = if (Test-Path -LiteralPath $vsDevCmd) {
    "`"$vsDevCmd`" -arch=$Platform -host_arch=$hostPlatform"
} else {
    "`"$vcvarsall`" $vcvarsPlatform"
}

$vcpkgPath = $null
$vcpkgCommand = Get-Command vcpkg -ErrorAction SilentlyContinue
if ($null -ne $vcpkgCommand) {
    $vcpkgPath = $vcpkgCommand.Source
}
elseif ($env:VCPKG_INSTALLATION_ROOT) {
    $candidate = Join-Path $env:VCPKG_INSTALLATION_ROOT "vcpkg.exe"
    if (Test-Path -LiteralPath $candidate) {
        $vcpkgPath = $candidate
    }
}

if ($null -ne $vcpkgPath) {
    & $vcpkgPath integrate install
    if ($LASTEXITCODE -ne 0) {
        throw "vcpkg integrate install failed. See winget-cli\doc\Developing.md for the required developer setup."
    }
}
else {
    Write-Warning "vcpkg was not found on PATH. If WinGetUtil build fails, follow winget-cli\doc\Developing.md and run 'vcpkg integrate install' from a Developer PowerShell prompt."
}

$vcpkgTriplet = Get-VcpkgTriplet -Platform $Platform -Configuration $Configuration
$vcpkgInstalledDir = Join-Path $wingetCliRoot "src\vcpkg_installed"
$tripletFile = Join-Path $vcpkgInstalledDir ".solution-triplet"
New-Item -ItemType Directory -Force -Path $vcpkgInstalledDir | Out-Null
Set-Content -LiteralPath $tripletFile -Value $vcpkgTriplet -NoNewline

$buildCommand = @(
    $developerEnvironment,
    "&&",
    "`"$msbuild`"",
    "`"$solutionPath`"",
    "/restore",
    "/m",
    "/nologo",
    "/verbosity:minimal",
    "/t:WinGetUtil",
    "/p:Configuration=$Configuration",
    "/p:Platform=$Platform",
    "/p:VcpkgTriplet=$vcpkgTriplet"
) -join " "

$wingetUtilPath = Join-Path $wingetCliRoot "src\$Platform\$Configuration\WinGetUtil\WinGetUtil.dll"
$buildSucceeded = $false

for ($attempt = 1; $attempt -le 2 -and -not $buildSucceeded; $attempt++) {
    cmd.exe /c $buildCommand
    if ($LASTEXITCODE -eq 0) {
        $buildSucceeded = $true
    }
}

if (-not $buildSucceeded) {
    throw "Failed to build WinGetUtil.dll from $solutionPath. Check winget-cli\doc\Developing.md for required Visual Studio workloads, Windows SDK, Developer Mode, and vcpkg setup."
}

if (-not (Test-Path -LiteralPath $wingetUtilPath)) {
    throw "Built WinGetUtil.dll was not found at $wingetUtilPath"
}

if ($Destination) {
    $destinationPath = $Destination
    New-Item -ItemType Directory -Force -Path $destinationPath | Out-Null
    Copy-Item -LiteralPath $wingetUtilPath -Destination (Join-Path $destinationPath "WinGetUtil.dll") -Force
}
