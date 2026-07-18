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
    [ValidateNotNullOrEmpty()]
    [string]$DestinationRoot = (Join-Path $PSScriptRoot '..\..'),

    [Parameter()]
    [string]$ArchivePath,

    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string]$CacheDirectory = (Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)) 'valorant-highlight-manager\ffmpeg-cache'),

    [Parameter()]
    [switch]$ValidationOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Assert-Condition {
    param(
        [Parameter(Mandatory)][bool]$Condition,
        [Parameter(Mandatory)][string]$Message
    )

    if (-not $Condition) {
        throw $Message
    }
}

function Assert-WindowsX64 {
    Assert-Condition -Condition $IsWindows -Message 'FFmpeg preparation is supported only on Windows.'
    Assert-Condition `
        -Condition ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -eq [System.Runtime.InteropServices.Architecture]::X64) `
        -Message 'FFmpeg preparation requires a native Windows x64 host.'
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

function Assert-NoReparseChain {
    param([Parameter(Mandatory)][string]$Path)

    $current = Get-FullNormalizedPath -Path $Path
    while (-not [string]::IsNullOrEmpty($current)) {
        if ([System.IO.Directory]::Exists($current) -or [System.IO.File]::Exists($current)) {
            $item = Get-Item -LiteralPath $current -Force
            Assert-Condition `
                -Condition (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -eq 0) `
                -Message "Path chain contains a symlink or reparse point: $current"
        }

        $parent = [System.IO.Directory]::GetParent($current)
        if ($null -eq $parent) {
            break
        }
        $current = $parent.FullName
    }
}

function Resolve-DestinationFile {
    param(
        [Parameter(Mandatory)][string]$Root,
        [Parameter(Mandatory)][string]$RelativePath
    )

    Assert-Condition -Condition (-not [System.IO.Path]::IsPathRooted($RelativePath)) -Message "Destination path must be relative: $RelativePath"
    $portablePath = $RelativePath.Replace('\', '/')
    $segments = $portablePath.Split('/')
    foreach ($segment in $segments) {
        Assert-Condition -Condition (-not [string]::IsNullOrWhiteSpace($segment)) -Message "Destination path contains an empty segment: $RelativePath"
        Assert-Condition -Condition ($segment -ne '.' -and $segment -ne '..') -Message "Destination path contains traversal: $RelativePath"
    }

    $rootFull = (Get-FullNormalizedPath -Path $Root).TrimEnd([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar)
    Assert-Condition -Condition ([System.IO.Directory]::Exists($rootFull)) -Message "Destination root must already exist: $rootFull"
    Assert-NoReparseChain -Path $rootFull

    $relativeNative = $portablePath.Replace('/', [System.IO.Path]::DirectorySeparatorChar)
    $candidate = Get-FullNormalizedPath -Path (Join-Path $rootFull $relativeNative)
    $prefix = $rootFull + [System.IO.Path]::DirectorySeparatorChar
    Assert-Condition -Condition $candidate.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase) -Message 'Destination escapes DestinationRoot.'

    $parentRelative = [System.IO.Path]::GetDirectoryName($relativeNative)
    $current = $rootFull
    if (-not [string]::IsNullOrWhiteSpace($parentRelative)) {
        foreach ($segment in $parentRelative.Split([System.IO.Path]::DirectorySeparatorChar)) {
            $current = Join-Path $current $segment
            if (-not [System.IO.Directory]::Exists($current)) {
                [void][System.IO.Directory]::CreateDirectory($current)
            }
            $item = Get-Item -LiteralPath $current -Force
            Assert-Condition -Condition $item.PSIsContainer -Message "Destination parent is not a directory: $current"
            Assert-Condition `
                -Condition (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -eq 0) `
                -Message "Destination parent is a symlink or reparse point: $current"
        }
    }

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

function Invoke-AtomicFileReplace {
    param(
        [Parameter(Mandatory)][string]$Source,
        [Parameter(Mandatory)][string]$Destination,
        [Parameter(Mandatory)][string]$Backup
    )

    $lastError = $null
    # Windows security scanners can briefly retain an image-section handle after
    # the smoke process exits. Keep replacement bounded while allowing that
    # transient handle to drain.
    for ($attempt = 1; $attempt -le 20; $attempt++) {
        try {
            [System.IO.File]::Replace($Source, $Destination, $Backup, $true)
            return
        }
        catch [System.IO.IOException] {
            $lastError = $_
            if ($attempt -lt 20) {
                Start-Sleep -Milliseconds 500
            }
        }
    }

    throw $lastError
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
    if ($source.redistributionReady -ne $true) { $issues.Add('sourceCompliance.redistributionReady is not true') }
    if ([string]$source.status -cne 'ready-for-redistribution') { $issues.Add("sourceCompliance.status is '$($source.status)', not 'ready-for-redistribution'") }
    if (-not (Test-HttpsUri -Value $Manifest.artifact.projectMirrorUrl)) { $issues.Add('artifact.projectMirrorUrl is missing') }
    if (-not (Test-HttpsUri -Value $source.binaryMirrorUrl)) { $issues.Add('sourceCompliance.binaryMirrorUrl is missing') }
    if ([string]$Manifest.artifact.projectMirrorUrl -cne [string]$source.binaryMirrorUrl) { $issues.Add('binary mirror URLs do not match') }
    if (-not (Test-HttpsUri -Value $source.correspondingSourceBundle.url)) { $issues.Add('correspondingSourceBundle.url is missing') }
    if ($null -eq $source.correspondingSourceBundle.sizeBytes -or [long]$source.correspondingSourceBundle.sizeBytes -le 0) { $issues.Add('correspondingSourceBundle.sizeBytes is missing') }
    if ([string]$source.correspondingSourceBundle.sha256 -notmatch '^[0-9a-f]{64}$') { $issues.Add('correspondingSourceBundle.sha256 is invalid') }
    if ($source.thirdPartyLicenseAuditComplete -ne $true) { $issues.Add('thirdPartyLicenseAuditComplete is not true') }
    if ($source.ijgAttributionIncluded -ne $true) { $issues.Add('ijgAttributionIncluded is not true') }
    if ([string]$source.patentReviewStatus -cnotin @('approved', 'not-required')) { $issues.Add("patentReviewStatus is '$($source.patentReviewStatus)'") }
    if ([string]::IsNullOrWhiteSpace([string]$source.legalApprovalReference)) { $issues.Add('legalApprovalReference is missing') }
    return $issues.ToArray()
}

function Assert-SafeArtifactUrl {
    param(
        [Parameter(Mandatory)][string]$Url,
        [Parameter(Mandatory)][string]$ExpectedFileName,
        [Parameter(Mandatory)][string]$ExpectedReleaseTag,
        [Parameter(Mandatory)][bool]$RequireProviderUrl
    )

    $uri = [System.Uri]$Url
    Assert-Condition -Condition ($uri.Scheme -ceq 'https') -Message 'FFmpeg artifact URL must use HTTPS.'
    Assert-Condition -Condition ([string]::IsNullOrEmpty($uri.UserInfo)) -Message 'FFmpeg artifact URL must not contain user information.'
    Assert-Condition -Condition ([string]::IsNullOrEmpty($uri.Query)) -Message 'FFmpeg artifact URL must not contain a query string.'
    Assert-Condition -Condition ([string]::IsNullOrEmpty($uri.Fragment)) -Message 'FFmpeg artifact URL must not contain a fragment.'

    if ($RequireProviderUrl) {
        $expectedPath = "/BtbN/FFmpeg-Builds/releases/download/$ExpectedReleaseTag/$ExpectedFileName"
        Assert-Condition -Condition ($uri.Host -ceq 'github.com') -Message 'ValidationOnly provider download must use github.com.'
        Assert-Condition -Condition ($uri.AbsolutePath -ceq $expectedPath) -Message 'Provider artifact URL does not match the pinned release tag and file name.'
    }
}

function Get-PinnedArchive {
    param(
        [Parameter(Mandatory)][string]$Url,
        [Parameter(Mandatory)][string]$CacheRoot,
        [Parameter(Mandatory)][string]$FileName,
        [Parameter(Mandatory)][long]$ExpectedSize,
        [Parameter(Mandatory)][string]$ExpectedSha256
    )

    $cacheFull = Get-FullNormalizedPath -Path $CacheRoot
    Assert-NoReparseChain -Path $cacheFull
    if (-not [System.IO.Directory]::Exists($cacheFull)) {
        [void][System.IO.Directory]::CreateDirectory($cacheFull)
    }
    Assert-NoReparseChain -Path $cacheFull
    $cacheItem = Get-Item -LiteralPath $cacheFull -Force
    Assert-Condition -Condition $cacheItem.PSIsContainer -Message "FFmpeg cache is not a directory: $cacheFull"

    $cachePath = Get-FullNormalizedPath -Path (Join-Path $cacheFull $FileName)
    $cachePrefix = $cacheFull.TrimEnd([System.IO.Path]::DirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar
    Assert-Condition -Condition $cachePath.StartsWith($cachePrefix, [System.StringComparison]::OrdinalIgnoreCase) -Message 'FFmpeg cache file escapes the cache directory.'

    if ([System.IO.File]::Exists($cachePath)) {
        Assert-ExpectedFile -Path $cachePath -ExpectedSize $ExpectedSize -ExpectedSha256 $ExpectedSha256 -Description 'cached FFmpeg archive'
        Write-Host "[ffmpeg] reusing verified cache archive $cachePath"
        return $cachePath
    }

    $partialPath = Join-Path $cacheFull (".$FileName.download-" + [Guid]::NewGuid().ToString('N'))
    $handler = [System.Net.Http.HttpClientHandler]::new()
    $handler.AllowAutoRedirect = $true
    $handler.MaxAutomaticRedirections = 5
    $handler.AutomaticDecompression = [System.Net.DecompressionMethods]::None
    $client = [System.Net.Http.HttpClient]::new($handler)
    $client.Timeout = [TimeSpan]::FromMinutes(20)
    $client.DefaultRequestHeaders.UserAgent.ParseAdd('valorant-highlight-manager-ffmpeg-preparer/1.0')

    $response = $null
    $networkStream = $null
    $fileStream = $null
    $incrementalHash = $null
    try {
        Write-Host "[ffmpeg] downloading pinned archive $Url"
        $response = $client.GetAsync([System.Uri]$Url, [System.Net.Http.HttpCompletionOption]::ResponseHeadersRead).GetAwaiter().GetResult()
        [void]$response.EnsureSuccessStatusCode()
        $finalUri = $response.RequestMessage.RequestUri
        $allowedFinalHosts = @('github.com', 'release-assets.githubusercontent.com', 'objects.githubusercontent.com')
        Assert-Condition -Condition ($finalUri.Scheme -ceq 'https') -Message "FFmpeg download redirected to a non-HTTPS URL: $finalUri"
        Assert-Condition -Condition ($finalUri.Host -cin $allowedFinalHosts) -Message "FFmpeg download redirected to an unapproved host: $($finalUri.Host)"
        if ($null -ne $response.Content.Headers.ContentLength) {
            Assert-Condition -Condition ([long]$response.Content.Headers.ContentLength -eq $ExpectedSize) -Message "FFmpeg download Content-Length mismatch: expected $ExpectedSize, found $($response.Content.Headers.ContentLength)."
        }

        $networkStream = $response.Content.ReadAsStream()
        $fileStream = [System.IO.FileStream]::new($partialPath, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write, [System.IO.FileShare]::None, 1048576, [System.IO.FileOptions]::SequentialScan)
        $incrementalHash = [System.Security.Cryptography.IncrementalHash]::CreateHash([System.Security.Cryptography.HashAlgorithmName]::SHA256)
        $buffer = [byte[]]::new(1048576)
        [long]$total = 0
        while (($read = $networkStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
            $total += $read
            Assert-Condition -Condition ($total -le $ExpectedSize) -Message "FFmpeg download exceeded the pinned size of $ExpectedSize bytes."
            $fileStream.Write($buffer, 0, $read)
            $incrementalHash.AppendData($buffer, 0, $read)
        }
        $fileStream.Flush($true)
        $fileStream.Dispose()
        $fileStream = $null
        Assert-Condition -Condition ($total -eq $ExpectedSize) -Message "FFmpeg download size mismatch: expected $ExpectedSize, found $total."
        $actualHash = [Convert]::ToHexString($incrementalHash.GetHashAndReset()).ToLowerInvariant()
        Assert-Condition -Condition ($actualHash -ceq $ExpectedSha256) -Message "FFmpeg download SHA-256 mismatch: expected $ExpectedSha256, found $actualHash."

        [System.IO.File]::Move($partialPath, $cachePath)
        Assert-ExpectedFile -Path $cachePath -ExpectedSize $ExpectedSize -ExpectedSha256 $ExpectedSha256 -Description 'downloaded FFmpeg archive'
        return $cachePath
    }
    finally {
        if ($null -ne $incrementalHash) { $incrementalHash.Dispose() }
        if ($null -ne $fileStream) { $fileStream.Dispose() }
        if ($null -ne $networkStream) { $networkStream.Dispose() }
        if ($null -ne $response) { $response.Dispose() }
        $client.Dispose()
        $handler.Dispose()
        if ([System.IO.File]::Exists($partialPath)) {
            [System.IO.File]::Delete($partialPath)
        }
    }
}

function Assert-SafeZipMemberName {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][string]$ExpectedRoot
    )

    Assert-Condition -Condition (-not [string]::IsNullOrWhiteSpace($Name)) -Message 'ZIP contains an empty member name.'
    Assert-Condition -Condition ($Name.IsNormalized([Text.NormalizationForm]::FormC)) -Message "ZIP member name is not Unicode NFC normalized: $Name"
    Assert-Condition -Condition (-not [regex]::IsMatch($Name, '[\x00-\x1f\x7f]')) -Message 'ZIP member name contains a control character.'
    Assert-Condition -Condition (-not $Name.Contains('\')) -Message "ZIP member name contains a backslash: $Name"
    Assert-Condition -Condition (-not $Name.StartsWith('/')) -Message "ZIP member name is absolute: $Name"
    Assert-Condition -Condition (-not $Name.Contains(':')) -Message "ZIP member name contains a colon: $Name"

    $isDirectory = $Name.EndsWith('/')
    $segments = $Name.Split('/')
    $segmentCount = if ($isDirectory) { $segments.Length - 1 } else { $segments.Length }
    Assert-Condition -Condition ($segmentCount -gt 0) -Message "ZIP member name has no path segments: $Name"
    for ($index = 0; $index -lt $segmentCount; $index++) {
        $segment = $segments[$index]
        Assert-Condition -Condition (-not [string]::IsNullOrEmpty($segment)) -Message "ZIP member name contains an empty path segment: $Name"
        Assert-Condition -Condition ($segment -ne '.' -and $segment -ne '..') -Message "ZIP member name contains traversal: $Name"
        Assert-Condition -Condition (-not [char]::IsWhiteSpace($segment[$segment.Length - 1]) -and -not $segment.EndsWith('.')) -Message "ZIP member name has a Windows-ambiguous suffix: $Name"
        Assert-Condition -Condition (-not [regex]::IsMatch($segment, '[<>"|?*]')) -Message "ZIP member name contains a Windows-invalid character: $Name"
        $baseName = $segment.Split('.')[0]
        Assert-Condition -Condition ($baseName -cnotmatch '^(?i:con|prn|aux|nul|com[1-9]|lpt[1-9])$') -Message "ZIP member uses a reserved Windows device name: $Name"
    }
    Assert-Condition -Condition ($segments[0] -ceq $ExpectedRoot) -Message "ZIP member is outside the pinned archive root: $Name"
}

function Assert-ZipMemberIsNotLink {
    param([Parameter(Mandatory)][System.IO.Compression.ZipArchiveEntry]$Entry)

    $external = ([int64]$Entry.ExternalAttributes) -band 0xffffffffL
    $unixType = ($external -shr 16) -band 0xF000
    $dosAttributes = $external -band 0xFFFF
    Assert-Condition -Condition ($unixType -ne 0xA000) -Message "ZIP contains a symbolic link: $($Entry.FullName)"
    Assert-Condition -Condition (($dosAttributes -band [int][System.IO.FileAttributes]::ReparsePoint) -eq 0) -Message "ZIP contains a reparse-point member: $($Entry.FullName)"
}

function Get-ZipEntryDigest {
    param(
        [Parameter(Mandatory)][System.IO.Compression.ZipArchiveEntry]$Entry,
        [Parameter(Mandatory)][long]$MaximumBytes
    )

    $stream = $Entry.Open()
    $hash = [System.Security.Cryptography.IncrementalHash]::CreateHash([System.Security.Cryptography.HashAlgorithmName]::SHA256)
    try {
        $buffer = [byte[]]::new(1048576)
        [long]$total = 0
        while (($read = $stream.Read($buffer, 0, $buffer.Length)) -gt 0) {
            $total += $read
            Assert-Condition -Condition ($total -le $MaximumBytes) -Message "ZIP member exceeded its allowed size: $($Entry.FullName)"
            $hash.AppendData($buffer, 0, $read)
        }
        return [pscustomobject]@{
            Size   = $total
            Sha256 = [Convert]::ToHexString($hash.GetHashAndReset()).ToLowerInvariant()
        }
    }
    finally {
        $hash.Dispose()
        $stream.Dispose()
    }
}

function Expand-PinnedFfmpeg {
    param(
        [Parameter(Mandatory)][string]$ZipPath,
        [Parameter(Mandatory)][object]$Manifest,
        [Parameter(Mandatory)][string]$StagingPath
    )

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [System.IO.Compression.ZipFile]::OpenRead($ZipPath)
    try {
        $entries = @($archive.Entries)
        Assert-Condition -Condition ($entries.Count -eq [int]$Manifest.artifact.entryCount) -Message "ZIP entry-count mismatch: expected $($Manifest.artifact.entryCount), found $($entries.Count)."
        Assert-Condition -Condition ($entries.Count -le 128) -Message 'ZIP contains too many members.'

        $seenNames = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
        [long]$totalUncompressed = 0
        $executableEntry = $null
        $licenseEntry = $null
        foreach ($entry in $entries) {
            Assert-SafeZipMemberName -Name $entry.FullName -ExpectedRoot ([string]$Manifest.artifact.archiveRoot)
            Assert-ZipMemberIsNotLink -Entry $entry
            Assert-Condition -Condition $seenNames.Add($entry.FullName) -Message "ZIP contains a case-insensitive duplicate member: $($entry.FullName)"
            Assert-Condition -Condition ($entry.Length -ge 0 -and $entry.Length -le 268435456) -Message "ZIP member has an unsafe declared size: $($entry.FullName)"
            if ($entry.FullName.EndsWith('/')) {
                Assert-Condition -Condition ($entry.Length -eq 0) -Message "ZIP directory member has non-zero length: $($entry.FullName)"
            }
            $totalUncompressed += $entry.Length
            Assert-Condition -Condition ($totalUncompressed -le 536870912) -Message 'ZIP declared uncompressed content exceeds 512 MiB.'

            if ($entry.FullName -ceq [string]$Manifest.artifact.executableMember) {
                Assert-Condition -Condition ($null -eq $executableEntry) -Message 'ZIP contains the FFmpeg executable member more than once.'
                $executableEntry = $entry
            }
            if ($entry.FullName -ceq [string]$Manifest.artifact.licenseMember) {
                Assert-Condition -Condition ($null -eq $licenseEntry) -Message 'ZIP contains the provider license member more than once.'
                $licenseEntry = $entry
            }
        }

        Assert-Condition -Condition ($totalUncompressed -eq [long]$Manifest.artifact.totalUncompressedBytes) -Message "ZIP uncompressed-size total mismatch: expected $($Manifest.artifact.totalUncompressedBytes), found $totalUncompressed."
        Assert-Condition -Condition ($null -ne $executableEntry) -Message 'ZIP does not contain the pinned FFmpeg executable member.'
        Assert-Condition -Condition ($null -ne $licenseEntry) -Message 'ZIP does not contain the pinned provider license member.'
        Assert-Condition -Condition ($executableEntry.Length -eq [long]$Manifest.artifact.executableSizeBytes) -Message 'ZIP FFmpeg executable declared size does not match the manifest.'
        Assert-Condition -Condition ($licenseEntry.Length -eq [long]$Manifest.artifact.licenseMemberSizeBytes) -Message 'ZIP provider license declared size does not match the manifest.'

        $licenseDigest = Get-ZipEntryDigest -Entry $licenseEntry -MaximumBytes ([long]$Manifest.artifact.licenseMemberSizeBytes)
        Assert-Condition -Condition ($licenseDigest.Size -eq [long]$Manifest.artifact.licenseMemberSizeBytes) -Message 'ZIP provider license extracted size does not match the manifest.'
        Assert-Condition -Condition ($licenseDigest.Sha256 -ceq [string]$Manifest.artifact.licenseMemberSha256) -Message 'ZIP provider license SHA-256 does not match the manifest.'

        $input = $executableEntry.Open()
        $output = [System.IO.FileStream]::new($StagingPath, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write, [System.IO.FileShare]::None, 1048576, [System.IO.FileOptions]::SequentialScan)
        $hash = [System.Security.Cryptography.IncrementalHash]::CreateHash([System.Security.Cryptography.HashAlgorithmName]::SHA256)
        try {
            $buffer = [byte[]]::new(1048576)
            [long]$written = 0
            while (($read = $input.Read($buffer, 0, $buffer.Length)) -gt 0) {
                $written += $read
                Assert-Condition -Condition ($written -le [long]$Manifest.artifact.executableSizeBytes) -Message 'Extracted FFmpeg executable exceeded the pinned size.'
                $output.Write($buffer, 0, $read)
                $hash.AppendData($buffer, 0, $read)
            }
            $output.Flush($true)
            Assert-Condition -Condition ($written -eq [long]$Manifest.artifact.executableSizeBytes) -Message 'Extracted FFmpeg executable size does not match the manifest.'
            $executableHash = [Convert]::ToHexString($hash.GetHashAndReset()).ToLowerInvariant()
            Assert-Condition -Condition ($executableHash -ceq [string]$Manifest.artifact.executableSha256) -Message 'Extracted FFmpeg executable SHA-256 does not match the manifest.'
        }
        finally {
            $hash.Dispose()
            $output.Dispose()
            $input.Dispose()
        }
    }
    finally {
        $archive.Dispose()
    }
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
Assert-Condition -Condition ([string]$manifest.artifact.fileName -ceq [System.IO.Path]::GetFileName([string]$manifest.artifact.fileName)) -Message 'FFmpeg artifact fileName must be a single safe path segment.'
Assert-Condition -Condition ([string]$manifest.artifact.fileName -cmatch '^[A-Za-z0-9][A-Za-z0-9._-]+\.zip$') -Message 'FFmpeg artifact fileName contains unsafe characters.'
Assert-Condition -Condition ([long]$manifest.artifact.sizeBytes -gt 0 -and [long]$manifest.artifact.sizeBytes -le 536870912) -Message 'FFmpeg archive size is outside the allowed range.'
Assert-Condition -Condition ([string]$manifest.artifact.sha256 -cmatch '^[0-9a-f]{64}$') -Message 'FFmpeg archive SHA-256 is invalid.'
Assert-Condition -Condition ([string]$manifest.artifact.executableSha256 -cmatch '^[0-9a-f]{64}$') -Message 'FFmpeg executable SHA-256 is invalid.'
Assert-Condition -Condition ([string]$manifest.artifact.licenseMemberSha256 -cmatch '^[0-9a-f]{64}$') -Message 'FFmpeg provider license SHA-256 is invalid.'

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

$downloadUrl = if ($ValidationOnly) { [string]$manifest.artifact.url } else { [string]$manifest.artifact.projectMirrorUrl }
Assert-SafeArtifactUrl `
    -Url $downloadUrl `
    -ExpectedFileName ([string]$manifest.artifact.fileName) `
    -ExpectedReleaseTag ([string]$manifest.provider.releaseTag) `
    -RequireProviderUrl ([bool]$ValidationOnly)

if ([string]::IsNullOrWhiteSpace($ArchivePath)) {
    $archiveFull = Get-PinnedArchive `
        -Url $downloadUrl `
        -CacheRoot $CacheDirectory `
        -FileName ([string]$manifest.artifact.fileName) `
        -ExpectedSize ([long]$manifest.artifact.sizeBytes) `
        -ExpectedSha256 ([string]$manifest.artifact.sha256)
}
else {
    $archiveFull = Get-FullNormalizedPath -Path $ArchivePath
    Assert-ExpectedFile `
        -Path $archiveFull `
        -ExpectedSize ([long]$manifest.artifact.sizeBytes) `
        -ExpectedSha256 ([string]$manifest.artifact.sha256) `
        -Description 'supplied FFmpeg archive'
    Write-Host "[ffmpeg] using verified supplied archive $archiveFull"
}

$destinationFull = Resolve-DestinationFile -Root $DestinationRoot -RelativePath ([string]$manifest.artifact.destination)
$destinationParent = [System.IO.Path]::GetDirectoryName($destinationFull)
$stagingPath = Join-Path $destinationParent ('.ffmpeg-stage-' + [Guid]::NewGuid().ToString('N') + '.exe')
$backupPath = Join-Path $destinationParent ('.ffmpeg-backup-' + [Guid]::NewGuid().ToString('N') + '.exe')
$installSucceeded = $false

try {
    Expand-PinnedFfmpeg -ZipPath $archiveFull -Manifest $manifest -StagingPath $stagingPath
    Assert-ExpectedFile `
        -Path $stagingPath `
        -ExpectedSize ([long]$manifest.artifact.executableSizeBytes) `
        -ExpectedSha256 ([string]$manifest.artifact.executableSha256) `
        -Description 'staged FFmpeg executable'

    $verifyScript = Join-Path $PSScriptRoot 'verify-ffmpeg.ps1'
    [void](Assert-RegularFile -Path $verifyScript -Description 'FFmpeg verification script')
    $verifyParameters = @{
        ManifestPath   = $manifestFull
        RepositoryRoot = $repositoryRootFull
        FfmpegPath     = $stagingPath
    }
    if ($ValidationOnly) {
        $verifyParameters.ValidationOnly = $true
    }
    & $verifyScript @verifyParameters

    if ([System.IO.File]::Exists($destinationFull)) {
        [void](Assert-RegularFile -Path $destinationFull -Description 'existing FFmpeg destination')
        $existingItem = Get-Item -LiteralPath $destinationFull -Force
        $existingHash = Get-Sha256Hex -Path $destinationFull
        if ($existingItem.Length -eq [long]$manifest.artifact.executableSizeBytes -and $existingHash -ceq [string]$manifest.artifact.executableSha256) {
            [System.IO.File]::Delete($stagingPath)
            Write-Host "[ffmpeg] destination already contains the pinned executable: $destinationFull"
        }
        else {
            Invoke-AtomicFileReplace -Source $stagingPath -Destination $destinationFull -Backup $backupPath
        }
    }
    else {
        [System.IO.File]::Move($stagingPath, $destinationFull)
    }

    Assert-ExpectedFile `
        -Path $destinationFull `
        -ExpectedSize ([long]$manifest.artifact.executableSizeBytes) `
        -ExpectedSha256 ([string]$manifest.artifact.executableSha256) `
        -Description 'installed FFmpeg executable'
    $installSucceeded = $true
    if ([System.IO.File]::Exists($backupPath)) {
        [System.IO.File]::Delete($backupPath)
    }
    Write-Host "[ffmpeg] prepared $destinationFull"
}
finally {
    if ([System.IO.File]::Exists($stagingPath)) {
        [System.IO.File]::Delete($stagingPath)
    }
    if ([System.IO.File]::Exists($backupPath)) {
        if ($installSucceeded) {
            [System.IO.File]::Delete($backupPath)
        }
        else {
            Write-Warning "FFmpeg install did not complete; preserving recovery backup: $backupPath"
        }
    }
}
