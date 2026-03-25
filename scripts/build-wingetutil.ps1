[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$WingetCliRoot,

    [string]$Configuration = "Debug",

    [string]$Platform = "x64",

    [string]$Destination
)

$ErrorActionPreference = "Stop"

$wingetCliRoot = (Resolve-Path -LiteralPath $WingetCliRoot -ErrorAction Stop).Path
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
$nuget = Join-Path $visualStudioPath "Common7\IDE\CommonExtensions\Microsoft\NuGet\NuGet.exe"

if (-not (Test-Path -LiteralPath $vsDevCmd) -and -not (Test-Path -LiteralPath $vcvarsall)) {
    throw "Neither VsDevCmd.bat nor vcvarsall.bat was found under $visualStudioPath"
}

if (-not (Test-Path -LiteralPath $msbuild)) {
    throw "MSBuild.exe was not found at $msbuild"
}

if (-not (Test-Path -LiteralPath $nuget)) {
    $nugetCommand = Get-Command nuget -ErrorAction SilentlyContinue
    if ($null -ne $nugetCommand) {
        $nuget = $nugetCommand.Source
    }
}

$developerEnvironment = if (Test-Path -LiteralPath $vsDevCmd) {
    "`"$vsDevCmd`" -arch=$Platform -host_arch=$Platform"
} else {
    "`"$vcvarsall`" $Platform"
}

$vcpkgCommand = Get-Command vcpkg -ErrorAction SilentlyContinue
if ($null -ne $vcpkgCommand) {
    & $vcpkgCommand.Source integrate install
    if ($LASTEXITCODE -ne 0) {
        throw "vcpkg integrate install failed. See winget-cli\doc\Developing.md for the required developer setup."
    }
}
else {
    Write-Warning "vcpkg was not found on PATH. If WinGetUtil build fails, follow winget-cli\doc\Developing.md and run 'vcpkg integrate install' from a Developer PowerShell prompt."
}

if (Test-Path -LiteralPath $nuget) {
    & $nuget restore $solutionPath -NonInteractive
    if ($LASTEXITCODE -ne 0) {
        throw "nuget restore failed for $solutionPath"
    }
}
else {
    Write-Warning "nuget.exe was not found. If package restore fails, follow winget-cli\doc\Developing.md and install the full Visual Studio/NuGet workload."
}

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
    "/p:Platform=$Platform"
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
