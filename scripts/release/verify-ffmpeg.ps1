#requires -Version 7.2

[CmdletBinding()]
param(
    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string]$ManifestPath = (Join-Path $PSScriptRoot '..\..\third_party\ffmpeg\windows-x64.json'),

    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string]$RepositoryRoot = (Join-Path $PSScriptRoot '..\..'),

    [Parameter()]
    [string]$FfmpegPath,

    [Parameter()]
    [switch]$ValidationOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Assert-Condition {
    param(
        [Parameter(Mandatory)]
        [bool]$Condition,

        [Parameter(Mandatory)]
        [string]$Message
    )

    if (-not $Condition) {
        throw $Message
    }
}

function Assert-StringSetEqual {
    param(
        [Parameter(Mandatory)][object[]]$Expected,
        [Parameter(Mandatory)][object[]]$Actual,
        [Parameter(Mandatory)][string]$Description
    )

    $expectedSet = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    $actualSet = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    foreach ($value in @($Expected)) { [void]$expectedSet.Add([string]$value) }
    foreach ($value in @($Actual)) { [void]$actualSet.Add([string]$value) }
    $noDuplicates = $expectedSet.Count -eq @($Expected).Count -and $actualSet.Count -eq @($Actual).Count
    Assert-Condition -Condition ($noDuplicates -and $expectedSet.SetEquals($actualSet)) -Message "$Description does not match the manifest."
}

function Assert-WindowsX64 {
    Assert-Condition -Condition $IsWindows -Message 'FFmpeg verification is supported only on Windows.'
    Assert-Condition `
        -Condition ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -eq [System.Runtime.InteropServices.Architecture]::X64) `
        -Message 'FFmpeg verification requires a native Windows x64 host.'
}

function Get-FullNormalizedPath {
    param([Parameter(Mandatory)][string]$Path)

    return [System.IO.Path]::GetFullPath($Path)
}

function Assert-RegularFile {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$Description
    )

    Assert-Condition -Condition ([System.IO.File]::Exists($Path)) -Message "$Description does not exist: $Path"
    $item = Get-Item -LiteralPath $Path -Force
    Assert-Condition -Condition (-not $item.PSIsContainer) -Message "$Description is not a regular file: $Path"
    Assert-Condition `
        -Condition (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -eq 0) `
        -Message "$Description must not be a symlink or reparse point: $Path"
    return $item
}

function Resolve-RepositoryFile {
    param(
        [Parameter(Mandatory)][string]$Root,
        [Parameter(Mandatory)][string]$RelativePath
    )

    Assert-Condition -Condition (-not [string]::IsNullOrWhiteSpace($RelativePath)) -Message 'Manifest contains an empty repository file path.'
    Assert-Condition -Condition (-not [System.IO.Path]::IsPathRooted($RelativePath)) -Message "Manifest repository path must be relative: $RelativePath"
    Assert-Condition -Condition ($RelativePath.IndexOf([char]0) -lt 0) -Message 'Manifest repository path contains a NUL character.'

    $segments = $RelativePath.Replace('\', '/').Split('/')
    foreach ($segment in $segments) {
        Assert-Condition -Condition (-not [string]::IsNullOrWhiteSpace($segment)) -Message "Manifest repository path contains an empty segment: $RelativePath"
        Assert-Condition -Condition ($segment -ne '.' -and $segment -ne '..') -Message "Manifest repository path contains traversal: $RelativePath"
    }

    $rootFull = (Get-FullNormalizedPath -Path $Root).TrimEnd([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar)
    $relativeNative = $RelativePath.Replace('/', [System.IO.Path]::DirectorySeparatorChar)
    $candidate = Get-FullNormalizedPath -Path (Join-Path $rootFull $relativeNative)
    $prefix = $rootFull + [System.IO.Path]::DirectorySeparatorChar
    $inside = $candidate.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)
    Assert-Condition -Condition $inside -Message "Manifest repository path escapes the repository root: $RelativePath"
    return $candidate
}

function Get-Sha256Hex {
    param([Parameter(Mandatory)][string]$Path)

    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Assert-ExpectedFile {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][long]$ExpectedSize,
        [Parameter(Mandatory)][string]$ExpectedSha256,
        [Parameter(Mandatory)][string]$Description
    )

    $item = Assert-RegularFile -Path $Path -Description $Description
    Assert-Condition -Condition ($item.Length -eq $ExpectedSize) -Message "$Description size mismatch: expected $ExpectedSize, found $($item.Length)."
    $actualHash = Get-Sha256Hex -Path $Path
    Assert-Condition -Condition ($actualHash -ceq $ExpectedSha256.ToLowerInvariant()) -Message "$Description SHA-256 mismatch: expected $ExpectedSha256, found $actualHash."
}

function Test-HttpsUri {
    param([AllowNull()][object]$Value)

    if ($null -eq $Value -or [string]::IsNullOrWhiteSpace([string]$Value)) {
        return $false
    }

    $uri = $null
    if (-not [System.Uri]::TryCreate([string]$Value, [System.UriKind]::Absolute, [ref]$uri)) {
        return $false
    }

    return $uri.Scheme -ceq 'https'
}

function Get-RedistributionGateIssues {
    param([Parameter(Mandatory)][object]$Manifest)

    $issues = [System.Collections.Generic.List[string]]::new()
    $source = $Manifest.sourceCompliance

    if ($source.redistributionReady -ne $true) {
        $issues.Add('sourceCompliance.redistributionReady is not true')
    }
    if ([string]$source.status -cne 'ready-for-redistribution') {
        $issues.Add("sourceCompliance.status is '$($source.status)', not 'ready-for-redistribution'")
    }
    if (-not (Test-HttpsUri -Value $Manifest.artifact.projectMirrorUrl)) {
        $issues.Add('artifact.projectMirrorUrl is not a pinned HTTPS project mirror URL')
    }
    if (-not (Test-HttpsUri -Value $source.binaryMirrorUrl)) {
        $issues.Add('sourceCompliance.binaryMirrorUrl is not a pinned HTTPS URL')
    }
    if (-not (Test-HttpsUri -Value $source.correspondingSourceBundle.url)) {
        $issues.Add('correspondingSourceBundle.url is not a pinned HTTPS URL')
    }
    if ($null -eq $source.correspondingSourceBundle.sizeBytes -or [long]$source.correspondingSourceBundle.sizeBytes -le 0) {
        $issues.Add('correspondingSourceBundle.sizeBytes is missing')
    }
    if ([string]$source.correspondingSourceBundle.sha256 -notmatch '^[0-9a-f]{64}$') {
        $issues.Add('correspondingSourceBundle.sha256 is not a lowercase SHA-256')
    }
    if ($source.thirdPartyLicenseAuditComplete -ne $true) {
        $issues.Add('thirdPartyLicenseAuditComplete is not true')
    }
    if ($source.ijgAttributionIncluded -ne $true) {
        $issues.Add('ijgAttributionIncluded is not true')
    }
    if ([string]$source.patentReviewStatus -cnotin @('approved', 'not-required')) {
        $issues.Add("patentReviewStatus is '$($source.patentReviewStatus)'")
    }
    if ([string]::IsNullOrWhiteSpace([string]$source.legalApprovalReference)) {
        $issues.Add('legalApprovalReference is missing')
    }

    return $issues.ToArray()
}

function Assert-PeX64Executable {
    param([Parameter(Mandatory)][string]$Path)

    $stream = [System.IO.File]::Open($Path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::Read)
    try {
        $reader = [System.IO.BinaryReader]::new($stream, [System.Text.Encoding]::ASCII, $true)
        try {
            Assert-Condition -Condition ($stream.Length -ge 64) -Message 'FFmpeg executable is too short to be a PE file.'
            Assert-Condition -Condition ($reader.ReadUInt16() -eq 0x5A4D) -Message 'FFmpeg executable is missing the DOS MZ signature.'
            $stream.Position = 0x3C
            $peOffset = $reader.ReadUInt32()
            Assert-Condition -Condition ($peOffset -le ($stream.Length - 6)) -Message 'FFmpeg executable has an invalid PE header offset.'
            $stream.Position = $peOffset
            Assert-Condition -Condition ($reader.ReadUInt32() -eq 0x00004550) -Message 'FFmpeg executable is missing the PE signature.'
            Assert-Condition -Condition ($reader.ReadUInt16() -eq 0x8664) -Message 'FFmpeg executable is not a Windows x64 PE image.'
        }
        finally {
            $reader.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}

function Invoke-CheckedProcess {
    param(
        [Parameter(Mandatory)][string]$Executable,
        [Parameter(Mandatory)][string[]]$Arguments,
        [Parameter(Mandatory)][int]$TimeoutSeconds,
        [Parameter(Mandatory)][string]$Description
    )

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Executable
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.WindowStyle = [System.Diagnostics.ProcessWindowStyle]::Hidden
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = [System.Text.Encoding]::UTF8
    $startInfo.StandardErrorEncoding = [System.Text.Encoding]::UTF8
    [void]$startInfo.Environment.Remove('FFREPORT')
    $startInfo.Environment['AV_LOG_FORCE_NOCOLOR'] = '1'
    foreach ($argument in $Arguments) {
        [void]$startInfo.ArgumentList.Add($argument)
    }

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        Assert-Condition -Condition $process.Start() -Message "Could not start $Description."
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            try {
                $process.Kill($true)
            }
            catch {
                Write-Verbose "Could not kill timed-out process: $($_.Exception.Message)"
            }
            $process.WaitForExit()
            throw "$Description exceeded the $TimeoutSeconds second timeout."
        }

        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        Assert-Condition -Condition ($process.ExitCode -eq 0) -Message "$Description failed with exit code $($process.ExitCode).`nSTDOUT:`n$stdout`nSTDERR:`n$stderr"
        return [pscustomobject]@{
            ExitCode = $process.ExitCode
            Stdout   = $stdout
            Stderr   = $stderr
            Combined = $stdout + "`n" + $stderr
        }
    }
    finally {
        $process.Dispose()
    }
}

function Assert-Capability {
    param(
        [Parameter(Mandatory)][string]$Kind,
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][string]$Text
    )

    $escaped = [System.Text.RegularExpressions.Regex]::Escape($Name)
    $pattern = switch ($Kind) {
        'protocols' { "(?m)^\s*$escaped\s*$" }
        'demuxers' { "(?m)^\s*D\s+(?:[A-Za-z0-9_]+,)*$escaped(?:,|\s)" }
        'decoders' { "(?m)^\s*[A-Z\.]{6}\s+$escaped\s" }
        'filters' { "(?m)^\s*[A-Z\.]+\s+$escaped\s" }
        'encoders' { "(?m)^\s*[A-Z\.]{6}\s+$escaped\s" }
        'muxers' { "(?m)^\s*E\s+$escaped\s" }
        default { throw "Unsupported capability kind: $Kind" }
    }

    Assert-Condition -Condition ([regex]::IsMatch($Text, $pattern)) -Message "FFmpeg is missing required $Kind capability '$Name'."
}

Assert-WindowsX64

$repositoryRootFull = Get-FullNormalizedPath -Path $RepositoryRoot
Assert-Condition -Condition ([System.IO.Directory]::Exists($repositoryRootFull)) -Message "Repository root does not exist: $repositoryRootFull"
$manifestFull = Get-FullNormalizedPath -Path $ManifestPath
[void](Assert-RegularFile -Path $manifestFull -Description 'FFmpeg manifest')
$manifest = Get-Content -LiteralPath $manifestFull -Raw -Encoding UTF8 | ConvertFrom-Json -Depth 100

Assert-Condition -Condition ([int]$manifest.schemaVersion -eq 1) -Message 'Unsupported FFmpeg manifest schemaVersion.'
Assert-Condition -Condition ([string]$manifest.platform -ceq 'windows') -Message 'FFmpeg manifest platform must be windows.'
Assert-Condition -Condition ([string]$manifest.architecture -ceq 'x86_64') -Message 'FFmpeg manifest architecture must be x86_64.'
Assert-Condition -Condition ([string]$manifest.artifact.archiveFormat -ceq 'zip') -Message 'FFmpeg artifact must be a ZIP archive.'
Assert-Condition -Condition ([string]$manifest.artifact.destination -ceq 'src-tauri/resources/bin/ffmpeg.exe') -Message 'FFmpeg destination must match the runtime resource contract.'
Assert-Condition -Condition ([string]$manifest.artifact.sha256 -cmatch '^[0-9a-f]{64}$') -Message 'Artifact SHA-256 must be 64 lowercase hexadecimal characters.'
Assert-Condition -Condition ([string]$manifest.artifact.executableSha256 -cmatch '^[0-9a-f]{64}$') -Message 'Executable SHA-256 must be 64 lowercase hexadecimal characters.'
Assert-Condition -Condition ([string]$manifest.ffmpeg.licenseExpression -ceq 'LGPL-3.0-or-later') -Message 'FFmpeg manifest must pin the LGPL-3.0-or-later variant.'

$mandatoryRequiredFlags = @('--enable-version3')
$mandatoryForbiddenFlags = @('--enable-gpl', '--enable-nonfree', '--enable-libx264', '--enable-libx265', '--enable-libfdk-aac')
$mandatoryDisabledFlags = @('--disable-libx264', '--disable-libx265', '--disable-libfdk-aac')
foreach ($flag in $mandatoryRequiredFlags) {
    Assert-Condition -Condition ([string]$flag -cin @($manifest.licensePolicy.requiredConfigureFlags)) -Message "Manifest license policy is missing mandatory required flag '$flag'."
}
foreach ($flag in $mandatoryForbiddenFlags) {
    Assert-Condition -Condition ([string]$flag -cin @($manifest.licensePolicy.forbiddenConfigureFlags)) -Message "Manifest license policy is missing mandatory forbidden flag '$flag'."
}
foreach ($flag in $mandatoryDisabledFlags) {
    Assert-Condition -Condition ([string]$flag -cin @($manifest.licensePolicy.requiredDisabledConfigureFlags)) -Message "Manifest license policy is missing mandatory disabled flag '$flag'."
}

$gateIssues = @(Get-RedistributionGateIssues -Manifest $manifest)
if ($gateIssues.Count -gt 0) {
    $gateMessage = "FFmpeg redistribution gate is closed:`n - " + ($gateIssues -join "`n - ")
    if ($ValidationOnly) {
        Write-Warning "$gateMessage`nValidationOnly permits local integrity/runtime validation, not redistribution."
    }
    else {
        throw $gateMessage
    }
}

if ([string]::IsNullOrWhiteSpace($FfmpegPath)) {
    $ffmpegFull = Resolve-RepositoryFile -Root $repositoryRootFull -RelativePath ([string]$manifest.artifact.destination)
}
else {
    $ffmpegFull = Get-FullNormalizedPath -Path $FfmpegPath
}

Assert-ExpectedFile `
    -Path $ffmpegFull `
    -ExpectedSize ([long]$manifest.artifact.executableSizeBytes) `
    -ExpectedSha256 ([string]$manifest.artifact.executableSha256) `
    -Description 'FFmpeg executable'
Assert-PeX64Executable -Path $ffmpegFull

$licensePaths = @{}
foreach ($licenseFile in @($manifest.licensePolicy.files)) {
    $relativePath = [string]$licenseFile.path
    $fullPath = Resolve-RepositoryFile -Root $repositoryRootFull -RelativePath $relativePath
    $item = Assert-RegularFile -Path $fullPath -Description "FFmpeg compliance file '$relativePath'"

    if ($licenseFile.PSObject.Properties.Name -contains 'sizeBytes') {
        Assert-Condition -Condition ($item.Length -eq [long]$licenseFile.sizeBytes) -Message "Compliance file size mismatch: $relativePath"
    }
    if ($licenseFile.PSObject.Properties.Name -contains 'sha256') {
        $actualHash = Get-Sha256Hex -Path $fullPath
        Assert-Condition -Condition ($actualHash -ceq ([string]$licenseFile.sha256).ToLowerInvariant()) -Message "Compliance file SHA-256 mismatch: $relativePath"
    }
    $licensePaths[$relativePath.Replace('\', '/')] = $fullPath
}

$lgplRelative = 'src-tauri/resources/licenses/ffmpeg/COPYING.LGPLv3.txt'
$gplRelative = 'src-tauri/resources/licenses/ffmpeg/COPYING.GPLv3.txt'
$buildInfoRelative = 'src-tauri/resources/licenses/ffmpeg/BUILD-INFO.json'
$sourceOfferRelative = 'src-tauri/resources/licenses/ffmpeg/SOURCE-OFFER.md'
foreach ($requiredPath in @($lgplRelative, $gplRelative, $buildInfoRelative, $sourceOfferRelative)) {
    Assert-Condition -Condition $licensePaths.ContainsKey($requiredPath) -Message "Manifest licensePolicy.files is missing $requiredPath."
}

$lgplHash = Get-Sha256Hex -Path $licensePaths[$lgplRelative]
Assert-Condition -Condition ($lgplHash -ceq [string]$manifest.artifact.licenseMemberSha256) -Message 'Tracked LGPL text does not match the provider archive license member hash.'
Assert-Condition -Condition ((Get-Item -LiteralPath $licensePaths[$lgplRelative]).Length -eq [long]$manifest.artifact.licenseMemberSizeBytes) -Message 'Tracked LGPL text does not match the provider archive license member size.'

$buildInfo = Get-Content -LiteralPath $licensePaths[$buildInfoRelative] -Raw -Encoding UTF8 | ConvertFrom-Json -Depth 100
Assert-Condition -Condition ([int]$buildInfo.schemaVersion -eq 1) -Message 'Unsupported FFmpeg BUILD-INFO schemaVersion.'
Assert-Condition -Condition ([string]$buildInfo.version -ceq [string]$manifest.ffmpeg.version) -Message 'BUILD-INFO version does not match the manifest.'
Assert-Condition -Condition ([string]$buildInfo.licenseExpression -ceq [string]$manifest.ffmpeg.licenseExpression) -Message 'BUILD-INFO license expression does not match the manifest.'
Assert-Condition -Condition ([string]$buildInfo.providerReleaseTag -ceq [string]$manifest.provider.releaseTag) -Message 'BUILD-INFO provider release does not match the manifest.'
Assert-Condition -Condition ([string]$buildInfo.providerBuildScriptsCommit -ceq [string]$manifest.provider.buildScriptsCommit) -Message 'BUILD-INFO build scripts commit does not match the manifest.'
Assert-Condition -Condition ([string]$buildInfo.ffmpegCommit -ceq [string]$manifest.ffmpeg.upstreamCommit) -Message 'BUILD-INFO FFmpeg commit does not match the manifest.'
Assert-Condition -Condition ([string]$buildInfo.compiler -ceq [string]$manifest.ffmpeg.compiler) -Message 'BUILD-INFO compiler does not match the manifest.'
Assert-Condition -Condition ([string]$buildInfo.archive.fileName -ceq [string]$manifest.artifact.fileName) -Message 'BUILD-INFO archive name does not match the manifest.'
Assert-Condition -Condition ([long]$buildInfo.archive.sizeBytes -eq [long]$manifest.artifact.sizeBytes) -Message 'BUILD-INFO archive size does not match the manifest.'
Assert-Condition -Condition ([string]$buildInfo.archive.sha256 -ceq [string]$manifest.artifact.sha256) -Message 'BUILD-INFO archive hash does not match the manifest.'
Assert-Condition -Condition ([string]$buildInfo.executable.pathInArchive -ceq [string]$manifest.artifact.executableMember) -Message 'BUILD-INFO executable member does not match the manifest.'
Assert-Condition -Condition ([long]$buildInfo.executable.sizeBytes -eq [long]$manifest.artifact.executableSizeBytes) -Message 'BUILD-INFO executable size does not match the manifest.'
Assert-Condition -Condition ([string]$buildInfo.executable.sha256 -ceq [string]$manifest.artifact.executableSha256) -Message 'BUILD-INFO executable hash does not match the manifest.'
Assert-Condition -Condition ([string]$buildInfo.redistributionStatus -ceq [string]$manifest.sourceCompliance.status) -Message 'BUILD-INFO redistribution status does not match the manifest.'
Assert-StringSetEqual -Expected @($manifest.licensePolicy.requiredConfigureFlags) -Actual @($buildInfo.configurePolicy.required) -Description 'BUILD-INFO required configure policy'
Assert-StringSetEqual -Expected @($manifest.licensePolicy.forbiddenConfigureFlags) -Actual @($buildInfo.configurePolicy.forbidden) -Description 'BUILD-INFO forbidden configure policy'
Assert-StringSetEqual -Expected @($manifest.licensePolicy.requiredDisabledConfigureFlags) -Actual @($buildInfo.configurePolicy.requiredDisabled) -Description 'BUILD-INFO required-disabled configure policy'

$sourceOffer = Get-Content -LiteralPath $licensePaths[$sourceOfferRelative] -Raw -Encoding UTF8
foreach ($traceValue in @(
        [string]$manifest.ffmpeg.upstreamCommit,
        [string]$manifest.provider.buildScriptsCommit,
        [string]$manifest.provider.releaseTag,
        [string]$manifest.sourceCompliance.status
    )) {
    Assert-Condition -Condition ($sourceOffer.Contains($traceValue, [System.StringComparison]::Ordinal)) -Message "SOURCE-OFFER.md is missing trace value '$traceValue'."
}

$timeoutSeconds = [int]$manifest.runtimeContract.processTimeoutSeconds
Assert-Condition -Condition ($timeoutSeconds -ge 1 -and $timeoutSeconds -le 120) -Message 'FFmpeg process timeout must be between 1 and 120 seconds.'

$versionResult = Invoke-CheckedProcess -Executable $ffmpegFull -Arguments @('-nostdin', '-hide_banner', '-version') -TimeoutSeconds $timeoutSeconds -Description 'FFmpeg version check'
Assert-Condition -Condition $versionResult.Stdout.StartsWith([string]$manifest.ffmpeg.versionPrefix, [System.StringComparison]::Ordinal) -Message 'FFmpeg version output does not match the pinned version prefix.'
Assert-Condition -Condition $versionResult.Combined.Contains("built with $($manifest.ffmpeg.compiler)", [System.StringComparison]::Ordinal) -Message 'FFmpeg compiler output does not match the pinned compiler.'

$buildResult = Invoke-CheckedProcess -Executable $ffmpegFull -Arguments @('-nostdin', '-hide_banner', '-buildconf') -TimeoutSeconds $timeoutSeconds -Description 'FFmpeg build configuration check'
$configureFlags = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
foreach ($match in [regex]::Matches($buildResult.Combined, '(?m)(?<!\S)--[^\s]+')) {
    [void]$configureFlags.Add($match.Value)
}
foreach ($requiredFlag in @($manifest.licensePolicy.requiredConfigureFlags)) {
    Assert-Condition -Condition $configureFlags.Contains([string]$requiredFlag) -Message "FFmpeg build is missing required configure flag '$requiredFlag'."
}
foreach ($requiredDisabledFlag in @($manifest.licensePolicy.requiredDisabledConfigureFlags)) {
    Assert-Condition -Condition $configureFlags.Contains([string]$requiredDisabledFlag) -Message "FFmpeg build is missing required disabled configure flag '$requiredDisabledFlag'."
}
foreach ($forbiddenFlag in @($manifest.licensePolicy.forbiddenConfigureFlags)) {
    Assert-Condition -Condition (-not $configureFlags.Contains([string]$forbiddenFlag)) -Message "FFmpeg build contains forbidden configure flag '$forbiddenFlag'."
}

$licenseResult = Invoke-CheckedProcess -Executable $ffmpegFull -Arguments @('-nostdin', '-hide_banner', '-L') -TimeoutSeconds $timeoutSeconds -Description 'FFmpeg runtime license check'
foreach ($requiredPhrase in @($manifest.licensePolicy.requiredLicensePhrases)) {
    Assert-Condition -Condition $licenseResult.Combined.Contains([string]$requiredPhrase, [System.StringComparison]::Ordinal) -Message "FFmpeg runtime license output is missing '$requiredPhrase'."
}

$capabilityCommands = [ordered]@{
    protocols = '-protocols'
    demuxers  = '-demuxers'
    decoders  = '-decoders'
    filters   = '-filters'
    encoders  = '-encoders'
    muxers    = '-muxers'
}
foreach ($kind in $capabilityCommands.Keys) {
    $capabilityResult = Invoke-CheckedProcess -Executable $ffmpegFull -Arguments @('-nostdin', '-hide_banner', $capabilityCommands[$kind]) -TimeoutSeconds $timeoutSeconds -Description "FFmpeg $kind check"
    foreach ($name in @($manifest.runtimeContract.requiredCapabilities.$kind)) {
        Assert-Capability -Kind $kind -Name ([string]$name) -Text $capabilityResult.Combined
    }
}

$fixture = $manifest.runtimeContract.smokeFixture
Assert-Condition -Condition ([string]$fixture.encoding -ceq 'base64') -Message 'Unsupported FFmpeg smoke fixture encoding.'
try {
    $fixtureBytes = [Convert]::FromBase64String([string]$fixture.base64)
}
catch {
    throw "FFmpeg smoke fixture is not valid base64: $($_.Exception.Message)"
}
Assert-Condition -Condition ($fixtureBytes.LongLength -eq [long]$fixture.sizeBytes) -Message 'FFmpeg smoke fixture size does not match the manifest.'
$fixtureHash = [Convert]::ToHexString([System.Security.Cryptography.SHA256]::HashData($fixtureBytes)).ToLowerInvariant()
Assert-Condition -Condition ($fixtureHash -ceq [string]$fixture.sha256) -Message 'FFmpeg smoke fixture SHA-256 does not match the manifest.'

$tempRoot = Get-FullNormalizedPath -Path ([System.IO.Path]::GetTempPath())
$smokeDirectory = Join-Path $tempRoot ("valorant-highlight-manager-ffmpeg-smoke-" + [Guid]::NewGuid().ToString('N'))
[void][System.IO.Directory]::CreateDirectory($smokeDirectory)
$inputPath = Join-Path $smokeDirectory 'synthetic.mp4'
$outputPath = Join-Path $smokeDirectory 'thumbnail.jpg'
try {
    [System.IO.File]::WriteAllBytes($inputPath, $fixtureBytes)
    $smokeArguments = [System.Collections.Generic.List[string]]::new()
    foreach ($argument in @($manifest.runtimeContract.thumbnailArguments)) {
        $resolvedArgument = switch ([string]$argument) {
            '{input}' { $inputPath }
            '{output}' { $outputPath }
            default { [string]$argument }
        }
        $smokeArguments.Add($resolvedArgument)
    }
    Assert-Condition -Condition ($smokeArguments.Contains($inputPath)) -Message 'Thumbnail runtime contract does not reference the smoke input.'
    Assert-Condition -Condition ($smokeArguments.Contains($outputPath)) -Message 'Thumbnail runtime contract does not reference the smoke output.'

    [void](Invoke-CheckedProcess -Executable $ffmpegFull -Arguments $smokeArguments.ToArray() -TimeoutSeconds $timeoutSeconds -Description 'FFmpeg synthetic MP4-to-JPEG smoke test')
    $outputItem = Assert-RegularFile -Path $outputPath -Description 'FFmpeg smoke-test JPEG'
    Assert-Condition -Condition ($outputItem.Length -ge 4) -Message 'FFmpeg smoke-test JPEG is too short.'
    Assert-Condition -Condition ($outputItem.Length -le [long]$manifest.runtimeContract.maximumOutputBytes) -Message 'FFmpeg smoke-test JPEG exceeds the runtime output limit.'
    $outputBytes = [System.IO.File]::ReadAllBytes($outputPath)
    Assert-Condition -Condition ($outputBytes[0] -eq 0xFF -and $outputBytes[1] -eq 0xD8) -Message 'FFmpeg smoke-test output is missing the JPEG SOI marker.'
    Assert-Condition -Condition ($outputBytes[$outputBytes.Length - 2] -eq 0xFF -and $outputBytes[$outputBytes.Length - 1] -eq 0xD9) -Message 'FFmpeg smoke-test output is missing the JPEG EOI marker.'
    $smokeHash = [Convert]::ToHexString([System.Security.Cryptography.SHA256]::HashData($outputBytes)).ToLowerInvariant()
    Write-Host "[ffmpeg] synthetic MP4 -> JPEG smoke passed ($($outputItem.Length) bytes, SHA-256 $smokeHash)."
}
finally {
    $smokeFull = Get-FullNormalizedPath -Path $smokeDirectory
    $tempPrefix = $tempRoot.TrimEnd([System.IO.Path]::DirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar
    $safeCleanup = $smokeFull.StartsWith($tempPrefix, [System.StringComparison]::OrdinalIgnoreCase) -and
        ([System.IO.Path]::GetFileName($smokeFull) -cmatch '^valorant-highlight-manager-ffmpeg-smoke-[0-9a-f]{32}$')
    if ($safeCleanup -and [System.IO.Directory]::Exists($smokeFull)) {
        Remove-Item -LiteralPath $smokeFull -Recurse -Force
    }
}

Write-Host "[ffmpeg] verified $ffmpegFull"
Write-Host "[ffmpeg] archive pin $($manifest.artifact.fileName): $($manifest.artifact.sha256)"
if ($gateIssues.Count -eq 0) {
    Write-Host '[ffmpeg] redistribution gate: ready.'
}
else {
    Write-Host '[ffmpeg] redistribution gate: closed (local ValidationOnly result).'
}
