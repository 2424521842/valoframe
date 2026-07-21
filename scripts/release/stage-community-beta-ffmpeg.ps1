#requires -Version 7.2

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateNotNullOrEmpty()]
    [string] $PackageRoot,

    [Parameter(Mandatory)]
    [ValidateNotNullOrEmpty()]
    [string] $BinaryArchiveOutputDirectory,

    [Parameter(Mandatory)]
    [ValidatePattern('^v[0-9]+\.[0-9]+\.[0-9]+-beta\.[1-9][0-9]*$')]
    [string] $ReleaseTag,

    [Parameter(Mandatory)]
    [ValidatePattern('^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$')]
    [string] $RepositorySlug,

    [Parameter(Mandatory)]
    [ValidatePattern('^[0-9a-f]{40}$')]
    [string] $ApplicationSourceCommit,

    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string] $RepositoryRoot = (Join-Path $PSScriptRoot '..\..'),

    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string] $ResourceRoot = (Join-Path $PSScriptRoot '..\..\src-tauri\resources'),

    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string] $OutputManifestPath = (Join-Path $PSScriptRoot '..\..\.tmp\community-beta\ffmpeg-windows-x64.json')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Assert-Condition {
    param([Parameter(Mandatory)] [bool] $Condition, [Parameter(Mandatory)] [string] $Message)
    if (-not $Condition) { throw $Message }
}

function Get-CanonicalDirectory {
    param([Parameter(Mandatory)] [string] $Path, [Parameter(Mandatory)] [string] $Description)
    $resolved = @(Resolve-Path -LiteralPath $Path -ErrorAction Stop)
    Assert-Condition -Condition ($resolved.Count -eq 1) -Message "$Description must resolve exactly once."
    $item = Get-Item -LiteralPath $resolved[0].ProviderPath -Force
    Assert-Condition -Condition ($item.PSIsContainer) -Message "$Description must be a directory."
    Assert-Condition -Condition (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -eq 0) -Message "$Description must not be a reparse point."
    return [System.IO.Path]::GetFullPath($item.FullName)
}

function Get-CanonicalFile {
    param([Parameter(Mandatory)] [string] $Path, [Parameter(Mandatory)] [string] $Description)
    $resolved = @(Resolve-Path -LiteralPath $Path -ErrorAction Stop)
    Assert-Condition -Condition ($resolved.Count -eq 1) -Message "$Description must resolve exactly once."
    $item = Get-Item -LiteralPath $resolved[0].ProviderPath -Force
    Assert-Condition -Condition (-not $item.PSIsContainer -and $item.Length -gt 0) -Message "$Description must be a non-empty file."
    Assert-Condition -Condition (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -eq 0) -Message "$Description must not be a reparse point."
    return [System.IO.Path]::GetFullPath($item.FullName)
}

function Get-Sha256 {
    param([Parameter(Mandatory)] [string] $Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Assert-PathWithinRoot {
    param([Parameter(Mandatory)] [string] $Root, [Parameter(Mandatory)] [string] $Path, [Parameter(Mandatory)] [string] $Description)
    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    $pathFull = [System.IO.Path]::GetFullPath($Path)
    $prefix = $rootFull + [System.IO.Path]::DirectorySeparatorChar
    Assert-Condition -Condition ($pathFull.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) -Message "$Description escapes its root."
}

function Test-PathWithinOrEqualRoot {
    param([Parameter(Mandatory)] [string] $Root, [Parameter(Mandatory)] [string] $Path)
    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    $pathFull = [System.IO.Path]::GetFullPath($Path).TrimEnd('\', '/')
    if ([string]::Equals($rootFull, $pathFull, [System.StringComparison]::OrdinalIgnoreCase)) {
        return $true
    }
    $prefix = $rootFull + [System.IO.Path]::DirectorySeparatorChar
    return $pathFull.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)
}

function Get-PackageFile {
    param([Parameter(Mandatory)] [string] $Root, [Parameter(Mandatory)] [string] $RelativePath, [Parameter(Mandatory)] [string] $Description)
    Assert-Condition -Condition (-not [System.IO.Path]::IsPathRooted($RelativePath) -and $RelativePath.IndexOf([char] 0) -lt 0) -Message "$Description path is unsafe."
    $segments = $RelativePath.Replace('\', '/').Split('/')
    Assert-Condition -Condition (@($segments | Where-Object { [string]::IsNullOrWhiteSpace($_) -or $_ -in @('.', '..') }).Count -eq 0) -Message "$Description path contains an unsafe segment."
    $candidate = [System.IO.Path]::GetFullPath((Join-Path $Root ($segments -join [System.IO.Path]::DirectorySeparatorChar)))
    Assert-PathWithinRoot -Root $Root -Path $candidate -Description $Description
    return Get-CanonicalFile -Path $candidate -Description $Description
}

function Invoke-CapturedProcess {
    param([Parameter(Mandatory)] [string] $Executable, [Parameter(Mandatory)] [string[]] $Arguments)
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Executable
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($argument in $Arguments) { [void] $startInfo.ArgumentList.Add($argument) }
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        Assert-Condition -Condition $process.Start() -Message 'Could not start staged FFmpeg.'
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(10000)) {
            try { $process.Kill($true) } catch { Write-Verbose $_ }
            throw 'Staged FFmpeg version inspection timed out.'
        }
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        Assert-Condition -Condition ($process.ExitCode -eq 0) -Message "Staged FFmpeg version inspection failed.`n$stdout`n$stderr"
        return "$stdout`n$stderr".Trim()
    }
    finally { $process.Dispose() }
}

function Write-Utf8Json {
    param([Parameter(Mandatory)] [object] $Value, [Parameter(Mandatory)] [string] $Path, [Parameter(Mandatory)] [int] $Depth)
    $Value | ConvertTo-Json -Depth $Depth | Set-Content -LiteralPath $Path -Encoding utf8
}

$repository = Get-CanonicalDirectory -Path $RepositoryRoot -Description 'repository root'
$package = Get-CanonicalDirectory -Path $PackageRoot -Description 'technical FFmpeg package root'
$resources = Get-CanonicalDirectory -Path $ResourceRoot -Description 'Tauri resource root'
Assert-PathWithinRoot -Root $repository -Path $resources -Description 'Tauri resource root'

$archiveOutputRequested = [System.IO.Path]::GetFullPath($BinaryArchiveOutputDirectory).TrimEnd('\', '/')
$archiveOutputName = [System.IO.Path]::GetFileName($archiveOutputRequested)
Assert-Condition -Condition ($archiveOutputName -cmatch '^[A-Za-z0-9][A-Za-z0-9._-]*$') -Message 'Binary archive output directory must have a safe final name.'
$archiveOutputParentRequested = [System.IO.Directory]::GetParent($archiveOutputRequested)
Assert-Condition -Condition ($null -ne $archiveOutputParentRequested) -Message 'Binary archive output directory must have an existing parent.'
$archiveOutputParent = Get-CanonicalDirectory -Path $archiveOutputParentRequested.FullName -Description 'binary archive output parent'
$archiveOutput = Join-Path $archiveOutputParent $archiveOutputName
Assert-Condition -Condition (-not (Test-Path -LiteralPath $archiveOutput)) -Message 'Binary archive output directory must not already exist.'
Assert-Condition -Condition (-not (Test-PathWithinOrEqualRoot -Root $package -Path $archiveOutput)) -Message 'Binary archive output directory must remain outside the hash-bound technical package root.'

$packageManifestPath = Get-PackageFile -Root $package -RelativePath 'COMMUNITY-BETA-FFMPEG-PACKAGE.json' -Description 'technical FFmpeg package manifest'
$packageChecksumPath = Get-PackageFile -Root $package -RelativePath 'SHA256SUMS.txt' -Description 'technical FFmpeg package checksum manifest'
$packageManifest = Get-Content -Raw -LiteralPath $packageManifestPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
Assert-Condition -Condition ([long] $packageManifest.schemaVersion -eq 1 -and [string] $packageManifest.status -ceq 'prepared-community-beta-candidate-not-public-release-approved') -Message 'Technical FFmpeg package status is invalid.'
Assert-Condition -Condition (
    [string] $packageManifest.channel -ceq 'community-beta' -and
    $packageManifest.technicalPromotion.communityBetaDistributionAuthorized -eq $false -and
    $packageManifest.technicalPromotion.publicReleaseApproved -eq $false
) -Message 'Technical FFmpeg package must remain unpromoted before the release-owner decision is applied.'
$technicalBoundary = $packageManifest.complianceBoundary
Assert-Condition -Condition (
    $technicalBoundary.technicalPackagingOnly -eq $true -and
    $technicalBoundary.communityBetaDistributionAuthorized -eq $false -and
    $technicalBoundary.publicReleaseApproved -eq $false -and
    $technicalBoundary.modifiesPublicReleasePolicy -eq $false -and
    $technicalBoundary.requiresReleaseOwnerApprovalBeforeDistribution -eq $true -and
    $technicalBoundary.requiresCodecPatentReviewBeforeStrictPublicRelease -eq $true -and
    $technicalBoundary.requiresToolchainRuntimeLicenseReviewBeforeStrictPublicRelease -eq $true
) -Message 'Technical FFmpeg package compliance boundary is missing or unsafe.'

$declaredChecksums = [System.Collections.Generic.Dictionary[string, string]]::new([System.StringComparer]::Ordinal)
foreach ($line in Get-Content -LiteralPath $packageChecksumPath -Encoding UTF8) {
    Assert-Condition -Condition ($line -cmatch '^([0-9a-f]{64})  ([A-Za-z0-9][A-Za-z0-9._/-]*)$') -Message 'Technical FFmpeg checksum manifest contains an invalid line.'
    $relative = [string] $Matches[2]
    Assert-Condition -Condition (-not $relative.Contains('..') -and $declaredChecksums.TryAdd($relative, [string] $Matches[1])) -Message 'Technical FFmpeg checksum manifest contains an unsafe or duplicate path.'
    $file = Get-PackageFile -Root $package -RelativePath $relative -Description "technical FFmpeg package file '$relative'"
    Assert-Condition -Condition ((Get-Sha256 -Path $file) -ceq [string] $Matches[1]) -Message "Technical FFmpeg package hash mismatch for '$relative'."
}
$actualPackageFiles = @(Get-ChildItem -LiteralPath $package -File -Recurse -Force | Where-Object { $_.FullName -cne $packageChecksumPath })
Assert-Condition -Condition ($actualPackageFiles.Count -eq $declaredChecksums.Count) -Message 'Technical FFmpeg checksum coverage is incomplete.'
foreach ($actualPackageFile in $actualPackageFiles) {
    $actualRelative = [System.IO.Path]::GetRelativePath($package, $actualPackageFile.FullName).Replace('\', '/')
    Assert-Condition -Condition ($declaredChecksums.ContainsKey($actualRelative)) -Message "Technical FFmpeg checksum manifest omits '$actualRelative'."
}

$ffmpegPath = Get-PackageFile -Root $package -RelativePath ([string] $packageManifest.executable.path) -Description 'verified minimal FFmpeg executable'
$sourceArchivePath = Get-PackageFile -Root $package -RelativePath ([string] $packageManifest.correspondingSource.path) -Description 'FFmpeg corresponding-source archive'
Assert-Condition -Condition ((Get-Sha256 -Path $ffmpegPath) -ceq [string] $packageManifest.executable.sha256 -and (Get-Item -LiteralPath $ffmpegPath).Length -eq [long] $packageManifest.executable.sizeBytes) -Message 'FFmpeg executable does not match the technical package manifest.'
Assert-Condition -Condition ((Get-Sha256 -Path $sourceArchivePath) -ceq [string] $packageManifest.correspondingSource.sha256 -and (Get-Item -LiteralPath $sourceArchivePath).Length -eq [long] $packageManifest.correspondingSource.sizeBytes) -Message 'FFmpeg corresponding source does not match the technical package manifest.'

$decisionPath = Get-CanonicalFile -Path (Join-Path $repository 'release\approvals\community-beta-v0.1.0.json') -Description 'Community Beta decision record'
$decision = Get-Content -Raw -LiteralPath $decisionPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
Assert-Condition -Condition ([long] $decision.schemaVersion -eq 1 -and [string] $decision.channel -ceq 'community-beta' -and $decision.strictPublicReleaseApproval -eq $false) -Message 'Community Beta decision record is invalid.'
foreach ($confirmation in @(
        'gameImagesMayBeDistributedInThisChannel',
        'projectBrandIconMayBeDistributedInThisChannel',
        'unofficialProjectDisclaimerApprovedForThisChannel',
        'ffmpegMinimalBuildMayBeDistributedInThisChannel',
        'codecPatentReviewDeferredToStrictRelease',
        'automaticUpdatesAreDisabled',
        'installerIsUnsigned',
        'ffmpegUseIsLimitedToThumbnailGeneration'
    )) {
    Assert-Condition -Condition ($decision.releaseOwnerConfirmations.$confirmation -eq $true) -Message "Community Beta decision is missing confirmation '$confirmation'."
}

$version = [string] $decision.version
Assert-Condition -Condition ($ReleaseTag -cmatch "^v$([regex]::Escape($version))-beta\.[1-9][0-9]*$") -Message 'Release tag does not match the approved Community Beta version.'
$candidateManifestPath = Get-CanonicalFile -Path (Join-Path $repository 'third_party\ffmpeg\minimal-windows-x64-candidate.json') -Description 'minimal FFmpeg candidate manifest'
$candidate = Get-Content -Raw -LiteralPath $candidateManifestPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
Assert-Condition -Condition ([string] $candidate.source.commit -ceq [string] $packageManifest.sourceCommit -and @($candidate.build.externalLibraries).Count -eq 0) -Message 'Technical package does not match the zero-external-library minimal candidate.'

$releaseAssets = Get-CanonicalDirectory -Path (Join-Path $package 'release-assets') -Description 'technical FFmpeg release assets directory'
$binaryArchiveName = 'valoframe-ffmpeg-minimal-windows-x64.zip'
[void] [System.IO.Directory]::CreateDirectory($archiveOutput)
$binaryArchivePath = Join-Path $archiveOutput $binaryArchiveName
Assert-Condition -Condition (-not (Test-Path -LiteralPath $binaryArchivePath)) -Message 'Community Beta FFmpeg binary archive output already exists.'
$releaseBaseUrl = "https://github.com/$RepositorySlug/releases/download/$ReleaseTag"
$binaryUrl = "$releaseBaseUrl/$binaryArchiveName"
$sourceArchiveName = [System.IO.Path]::GetFileName($sourceArchivePath)
$sourceUrl = "$releaseBaseUrl/$sourceArchiveName"
$sourceArchiveItem = Get-Item -LiteralPath $sourceArchivePath -Force
$sourceArchiveHash = Get-Sha256 -Path $sourceArchivePath
$binaryArchiveNotice = @"
# FFmpeg Community Beta binary archive notice

This ZIP contains the minimal Windows x64 FFmpeg executable used by VALOFRAME,
the GNU LGPLv3 and GPLv3 license texts, and its recorded build evidence. FFmpeg
is executed as a separate command-line program for local thumbnail generation.

The exact corresponding source is not stored inside this binary ZIP. It is
published at no additional charge as a sidecar asset on the same GitHub release:

- Sidecar file: $sourceArchiveName
- Sidecar URL: $sourceUrl
- Sidecar size: $($sourceArchiveItem.Length) bytes
- Sidecar SHA-256: $sourceArchiveHash
- FFmpeg commit: $($candidate.source.commit)

This software is based in part on the work of the Independent JPEG Group.

FFmpeg is a trademark of Fabrice Bellard, originator of the FFmpeg project. The
FFmpeg project does not endorse VALOFRAME.

This archive is authorized only by the repository release-owner decision for
the documented unsigned Community Beta. It does not grant strict public-release
approval; codec-patent and toolchain-runtime-license reviews remain pending for
that separate channel.
"@

$archiveInputs = [System.Collections.Generic.List[object]]::new()
$archiveInputs.Add([ordered]@{ path = 'bin/ffmpeg.exe'; source = $ffmpegPath })
foreach ($name in @('COPYING.LGPLv3.txt', 'COPYING.GPLv3.txt')) {
    $archiveInputs.Add([ordered]@{
            path = "licenses/$name"
            source = Get-PackageFile -Root $package -RelativePath "installer-resource-overlay/licenses/ffmpeg/$name" -Description "FFmpeg archive license '$name'"
        })
}
$archiveInputs.Add([ordered]@{ path = 'licenses/THIRD-PARTY-NOTICE.md'; text = $binaryArchiveNotice })
$buildEvidenceRoot = Get-CanonicalDirectory -Path (Join-Path $releaseAssets 'build-evidence') -Description 'FFmpeg build evidence directory'
foreach ($item in @(Get-ChildItem -LiteralPath $buildEvidenceRoot -File -Force | Sort-Object Name)) {
    Assert-Condition -Condition (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -eq 0) -Message 'FFmpeg build evidence must not contain reparse points.'
    $archiveInputs.Add([ordered]@{ path = "build-evidence/$($item.Name)"; source = $item.FullName })
}
$archiveInputs.Add([ordered]@{ path = 'WINDOWS-VERIFICATION.json'; source = (Get-PackageFile -Root $package -RelativePath 'release-assets/WINDOWS-VERIFICATION.json' -Description 'Windows FFmpeg verification') })
$archiveInputs.Add([ordered]@{ path = 'COMMUNITY-BETA-FFMPEG-PACKAGE.json'; source = $packageManifestPath })

Add-Type -AssemblyName System.IO.Compression
$archiveStream = [System.IO.File]::Open($binaryArchivePath, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None)
try {
    $zip = [System.IO.Compression.ZipArchive]::new($archiveStream, [System.IO.Compression.ZipArchiveMode]::Create, $true)
    try {
        foreach ($archiveInput in @($archiveInputs | Sort-Object path)) {
            $entry = $zip.CreateEntry([string] $archiveInput.path, [System.IO.Compression.CompressionLevel]::Optimal)
            $entry.LastWriteTime = [DateTimeOffset]::new(2026, 1, 1, 0, 0, 0, [TimeSpan]::Zero)
            $entryStream = $entry.Open()
            try {
                if ($archiveInput.Contains('source')) {
                    $inputStream = [System.IO.File]::OpenRead([string] $archiveInput.source)
                    try { $inputStream.CopyTo($entryStream) }
                    finally { $inputStream.Dispose() }
                }
                else {
                    Assert-Condition -Condition ($archiveInput.Contains('text')) -Message "Archive entry '$($archiveInput.path)' has no source or text."
                    $text = ([string] $archiveInput.text).Replace("`r`n", "`n").Replace("`r", "`n")
                    if (-not $text.EndsWith("`n", [System.StringComparison]::Ordinal)) { $text += "`n" }
                    $bytes = [System.Text.UTF8Encoding]::new($false).GetBytes($text)
                    $entryStream.Write($bytes, 0, $bytes.Length)
                }
            }
            finally { $entryStream.Dispose() }
        }
    }
    finally { $zip.Dispose() }
}
finally { $archiveStream.Dispose() }
$binaryArchive = Get-CanonicalFile -Path $binaryArchivePath -Description 'Community Beta FFmpeg binary archive'

$versionOutput = Invoke-CapturedProcess -Executable $ffmpegPath -Arguments @('-nostdin', '-hide_banner', '-version')
$versionLine = @($versionOutput -split '\r?\n' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })[0]
Assert-Condition -Condition ($versionLine -cmatch '^ffmpeg version (?<version>\S+) Copyright') -Message 'Could not parse the minimal FFmpeg version line.'
$ffmpegVersion = [string] $Matches['version']
$buildMetadataPath = Get-PackageFile -Root $package -RelativePath 'release-assets/build-evidence/BUILD-METADATA.json' -Description 'FFmpeg build metadata'
$buildMetadata = Get-Content -Raw -LiteralPath $buildMetadataPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
$compilerPath = Get-PackageFile -Root $package -RelativePath 'release-assets/build-evidence/compiler-version.txt' -Description 'FFmpeg compiler version'
$compiler = @((Get-Content -LiteralPath $compilerPath -Encoding UTF8) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })[0].Trim()

$redistributionStatus = 'community-beta-source-bundled-formal-review-pending'

$licenseRoot = Join-Path $resources 'licenses\ffmpeg'
$binRoot = Join-Path $resources 'bin'
[void] [System.IO.Directory]::CreateDirectory($licenseRoot)
[void] [System.IO.Directory]::CreateDirectory($binRoot)
Copy-Item -LiteralPath $ffmpegPath -Destination (Join-Path $binRoot 'ffmpeg.exe') -Force
foreach ($name in @('COPYING.LGPLv3.txt', 'COPYING.GPLv3.txt')) {
    $sourceLicense = Get-PackageFile -Root $package -RelativePath "installer-resource-overlay/licenses/ffmpeg/$name" -Description "FFmpeg license '$name'"
    Copy-Item -LiteralPath $sourceLicense -Destination (Join-Path $licenseRoot $name) -Force
}

$buildInfo = [ordered]@{
    schemaVersion = 1
    component = 'FFmpeg'
    version = $ffmpegVersion
    licenseExpression = [string] $candidate.build.licenseExpression
    provider = 'VALOFRAME source-pinned minimal FFmpeg build'
    providerReleaseTag = $ReleaseTag
    providerBuildScriptsCommit = $ApplicationSourceCommit
    ffmpegCommit = [string] $candidate.source.commit
    compiler = $compiler
    variant = 'windows-x64-minimal-lgpl-static'
    releaseChannel = 'community-beta'
    strictPublicReleaseApproved = $false
    archive = [ordered]@{
        fileName = $binaryArchiveName
        sizeBytes = (Get-Item -LiteralPath $binaryArchive).Length
        sha256 = Get-Sha256 -Path $binaryArchive
    }
    executable = [ordered]@{
        pathInArchive = 'bin/ffmpeg.exe'
        sizeBytes = (Get-Item -LiteralPath $ffmpegPath).Length
        sha256 = Get-Sha256 -Path $ffmpegPath
    }
    configurePolicy = [ordered]@{
        required = @($candidate.build.configureFlags)
        requiredDisabled = @()
        forbidden = @($candidate.build.forbiddenFlags)
    }
    redistributionStatus = $redistributionStatus
}
$buildInfoPath = Join-Path $licenseRoot 'BUILD-INFO.json'
Write-Utf8Json -Value $buildInfo -Path $buildInfoPath -Depth 20

$sourceOffer = @"
# FFmpeg source availability for $ReleaseTag

This Community Beta invokes the bundled minimal FFmpeg executable as a separate
command-line program solely to generate local video thumbnails. The executable
is licensed under LGPL-3.0-or-later.

- Binary and build evidence: $binaryUrl
- Exact corresponding source: $sourceUrl
- Corresponding-source size: $((Get-Item -LiteralPath $sourceArchivePath).Length) bytes
- Corresponding-source SHA-256: $(Get-Sha256 -Path $sourceArchivePath)
- FFmpeg commit: $($candidate.source.commit)
- Application build commit: $ApplicationSourceCommit

The corresponding-source archive contains the exact FFmpeg source tree, pinned
build contract, build script, configuration and build metadata used for this
binary. It is published beside the installer at no additional charge.

This record authorizes only the documented unsigned Community Beta channel. It
does not mark the repository's strict public-release policy as approved; formal
signing, timestamp, VM and codec-review gates remain separately tracked.
"@
$sourceOfferPath = Join-Path $licenseRoot 'SOURCE-OFFER.md'
$sourceOffer | Set-Content -LiteralPath $sourceOfferPath -Encoding utf8

$installedNotice = @"
# FFmpeg Community Beta component notice

This installation includes a minimal Windows x64 FFmpeg executable used only as
a separate command-line program for local thumbnail generation. FFmpeg is
provided under LGPL-3.0-or-later; the accompanying license texts are installed
in this directory.

The exact corresponding source is published at no additional charge beside the
installer as `$sourceArchiveName`:

- Source URL: $sourceUrl
- Source size: $($sourceArchiveItem.Length) bytes
- Source SHA-256: $sourceArchiveHash
- FFmpeg commit: $($candidate.source.commit)

This software is based in part on the work of the Independent JPEG Group.

FFmpeg is a trademark of Fabrice Bellard, originator of the FFmpeg project. The
FFmpeg project does not endorse VALOFRAME.
"@
$installedNoticePath = Join-Path $licenseRoot 'THIRD-PARTY-NOTICE.md'
$installedNotice | Set-Content -LiteralPath $installedNoticePath -Encoding utf8

$licenseFiles = @(
    [ordered]@{ path = 'src-tauri/resources/licenses/ffmpeg/COPYING.LGPLv3.txt'; local = (Join-Path $licenseRoot 'COPYING.LGPLv3.txt'); pin = $true },
    [ordered]@{ path = 'src-tauri/resources/licenses/ffmpeg/COPYING.GPLv3.txt'; local = (Join-Path $licenseRoot 'COPYING.GPLv3.txt'); pin = $true },
    [ordered]@{ path = 'src-tauri/resources/licenses/ffmpeg/BUILD-INFO.json'; local = $buildInfoPath; pin = $false },
    [ordered]@{ path = 'src-tauri/resources/licenses/ffmpeg/SOURCE-OFFER.md'; local = $sourceOfferPath; pin = $false },
    [ordered]@{ path = 'src-tauri/resources/licenses/ffmpeg/THIRD-PARTY-NOTICE.md'; local = $installedNoticePath; pin = $false }
)
$licenseDeclarations = foreach ($licenseFile in $licenseFiles) {
    $record = [ordered]@{ path = $licenseFile.path }
    if ($licenseFile.pin) {
        $record['sizeBytes'] = (Get-Item -LiteralPath $licenseFile.local).Length
        $record['sha256'] = Get-Sha256 -Path $licenseFile.local
    }
    $record
}

$manifest = [ordered]@{
    schemaVersion = 2
    releaseChannel = 'community-beta'
    productionPromotionAuthorized = $false
    platform = 'windows'
    architecture = 'x86_64'
    provider = [ordered]@{
        name = 'VALOFRAME source-pinned minimal FFmpeg build'
        repositoryUrl = "https://github.com/$RepositorySlug"
        officialListingUrl = 'https://ffmpeg.org/download.html#build-windows'
        releaseTag = $ReleaseTag
        releaseUrl = "https://github.com/$RepositorySlug/releases/tag/$ReleaseTag"
        buildScriptsCommit = $ApplicationSourceCommit
        buildScriptsCommitUrl = "https://github.com/$RepositorySlug/commit/$ApplicationSourceCommit"
    }
    artifact = [ordered]@{
        fileName = $binaryArchiveName
        url = $binaryUrl
        projectMirrorUrl = $binaryUrl
        sizeBytes = (Get-Item -LiteralPath $binaryArchive).Length
        sha256 = Get-Sha256 -Path $binaryArchive
        archiveFormat = 'zip'
        archiveRoot = '.'
        entryCount = $archiveInputs.Count
        executableMember = 'bin/ffmpeg.exe'
        executableSizeBytes = (Get-Item -LiteralPath $ffmpegPath).Length
        executableSha256 = Get-Sha256 -Path $ffmpegPath
        licenseMember = 'licenses/COPYING.LGPLv3.txt'
        destination = 'src-tauri/resources/bin/ffmpeg.exe'
    }
    ffmpeg = [ordered]@{
        version = $ffmpegVersion
        versionPrefix = $versionLine
        upstreamCommit = [string] $candidate.source.commit
        upstreamCommitUrl = [string] $candidate.source.commitUrl
        compiler = $compiler
        licenseExpression = [string] $candidate.build.licenseExpression
    }
    build = [ordered]@{
        applicationSourceCommit = $ApplicationSourceCommit
        targetTriple = [string] $candidate.build.targetTriple
        configureFlags = @($candidate.build.configureFlags)
        externalLibraries = @()
    }
    licensePolicy = [ordered]@{
        requiredConfigureFlags = @($candidate.build.configureFlags)
        forbiddenConfigureFlags = @($candidate.build.forbiddenFlags)
        requiredDisabledConfigureFlags = @()
        files = @($licenseDeclarations)
    }
    runtimeContract = $candidate.runtimeContract
    sourceCompliance = [ordered]@{
        redistributionReady = $false
        status = $redistributionStatus
        binaryMirrorUrl = $binaryUrl
        correspondingSourceBundle = [ordered]@{
            url = $sourceUrl
            sizeBytes = (Get-Item -LiteralPath $sourceArchivePath).Length
            sha256 = Get-Sha256 -Path $sourceArchivePath
        }
        upstreamSource = [ordered]@{
            commit = [string] $candidate.source.commit
            referenceUrl = [string] $candidate.source.referenceArchive
        }
        buildScriptsSource = [ordered]@{
            commit = $ApplicationSourceCommit
            referenceUrl = "https://github.com/$RepositorySlug/archive/$ApplicationSourceCommit.tar.gz"
        }
        ffmpegExternalLibraryAuditComplete = $true
        thirdPartyLicenseAuditComplete = $false
        toolchainRuntimeLicenseReviewStatus = 'pending-for-strict-public-release'
        ijgAttributionRequired = $true
        ijgAttributionIncluded = $true
        patentReviewStatus = 'pending-for-strict-public-release'
        legalApprovalReference = $null
        requiredBeforeStrictPublicRelease = @(
            'Complete the MinGW/toolchain runtime license review.',
            'Complete the target-market codec review.',
            'Complete Authenticode signing and trusted timestamping.',
            'Complete the clean-VM and data-safety evidence matrices.'
        )
    }
}

$outputFull = if ([System.IO.Path]::IsPathRooted($OutputManifestPath)) {
    [System.IO.Path]::GetFullPath($OutputManifestPath)
}
else {
    [System.IO.Path]::GetFullPath((Join-Path $repository $OutputManifestPath))
}
Assert-PathWithinRoot -Root $repository -Path $outputFull -Description 'Community Beta FFmpeg manifest output'
Assert-Condition -Condition (-not (Test-Path -LiteralPath $outputFull)) -Message 'Community Beta FFmpeg manifest output already exists.'
$outputParent = [System.IO.Directory]::GetParent($outputFull).FullName
[void] [System.IO.Directory]::CreateDirectory($outputParent)
Write-Utf8Json -Value $manifest -Path $outputFull -Depth 100

[ordered]@{
    schemaVersion = 1
    status = 'staged-for-unsigned-community-beta'
    strictPublicReleaseApproved = $false
    releaseTag = $ReleaseTag
    applicationSourceCommit = $ApplicationSourceCommit
    manifestPath = $outputFull
    manifestSha256 = Get-Sha256 -Path $outputFull
    executable = [ordered]@{
        path = (Join-Path $binRoot 'ffmpeg.exe')
        sizeBytes = (Get-Item -LiteralPath (Join-Path $binRoot 'ffmpeg.exe')).Length
        sha256 = Get-Sha256 -Path (Join-Path $binRoot 'ffmpeg.exe')
    }
    binaryArchive = [ordered]@{
        path = $binaryArchive
        sizeBytes = (Get-Item -LiteralPath $binaryArchive).Length
        sha256 = Get-Sha256 -Path $binaryArchive
    }
    correspondingSource = [ordered]@{
        path = $sourceArchivePath
        sizeBytes = (Get-Item -LiteralPath $sourceArchivePath).Length
        sha256 = Get-Sha256 -Path $sourceArchivePath
    }
} | ConvertTo-Json -Depth 10
