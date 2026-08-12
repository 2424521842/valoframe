#requires -Version 7.2

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateNotNullOrEmpty()]
    [string] $ArchivePath,

    [Parameter(Mandatory)]
    [ValidatePattern('^[0-9A-Fa-f]{64}$')]
    [string] $ExpectedArchiveSha256,

    [Parameter(Mandatory)]
    [ValidatePattern('^[0-9A-Fa-f]{40}$')]
    [string] $ExpectedSourceCommit,

    [Parameter(Mandatory)]
    [ValidateNotNullOrEmpty()]
    [string] $OutputDirectory,

    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string] $PolicyPath = (Join-Path $PSScriptRoot '..\..\release\public-release-policy.json'),

    [Parameter()]
    [string] $ReportPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$MaxEvidenceEntryCount = 4096L
$MaxEvidenceUncompressedBytes = 2147483648L

function Get-RegularFile {
    param(
        [Parameter(Mandatory)] [string] $LiteralPath,
        [Parameter(Mandatory)] [string] $Description
    )

    $resolved = @(Resolve-Path -LiteralPath $LiteralPath -ErrorAction Stop)
    if ($resolved.Count -ne 1) {
        throw "$Description must resolve to exactly one path."
    }
    $item = Get-Item -LiteralPath $resolved[0].ProviderPath -Force
    if ($item.PSIsContainer -or ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$Description must be a regular file without a reparse point."
    }
    return $item
}

function Assert-NoReparseDirectoryChain {
    param([Parameter(Mandatory)] [string] $LiteralPath)

    $current = [System.IO.Path]::GetFullPath($LiteralPath)
    while (-not [string]::IsNullOrWhiteSpace($current)) {
        if ([System.IO.Directory]::Exists($current)) {
            $item = Get-Item -LiteralPath $current -Force
            if (-not $item.PSIsContainer -or ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "Evidence path chain contains a non-directory or reparse point: '$current'."
            }
        }
        $parent = [System.IO.Directory]::GetParent($current)
        if ($null -eq $parent) { break }
        $current = $parent.FullName
    }
}

function Get-SafeRelativeSegments {
    param(
        [Parameter(Mandatory)] [string] $RelativePath,
        [Parameter(Mandatory)] [string] $Description,
        [switch] $AllowTrailingSlash
    )

    if ([string]::IsNullOrWhiteSpace($RelativePath) -or
        [System.IO.Path]::IsPathRooted($RelativePath) -or
        $RelativePath.Contains('\') -or $RelativePath.Contains(':') -or
        $RelativePath.IndexOf([char] 0) -ge 0) {
        throw "$Description is not a safe portable relative path: '$RelativePath'."
    }
    $portable = if ($AllowTrailingSlash) { $RelativePath.TrimEnd('/') } else { $RelativePath }
    if ([string]::IsNullOrWhiteSpace($portable)) {
        throw "$Description cannot name the archive root."
    }
    $segments = @($portable.Split('/'))
    if (@($segments | Where-Object { [string]::IsNullOrWhiteSpace($_) -or $_ -in @('.', '..') }).Count -ne 0) {
        throw "$Description contains an unsafe path segment: '$RelativePath'."
    }
    return $segments
}

function Resolve-StagedEvidenceFile {
    param(
        [Parameter(Mandatory)] [string] $Root,
        [Parameter(Mandatory)] [AllowNull()] [AllowEmptyString()] [string] $RelativePath,
        [Parameter(Mandatory)] [string] $Description
    )

    $segments = Get-SafeRelativeSegments -RelativePath $RelativePath -Description $Description
    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    $candidate = [System.IO.Path]::GetFullPath((Join-Path $rootFull ($segments -join [System.IO.Path]::DirectorySeparatorChar)))
    $prefix = $rootFull + [System.IO.Path]::DirectorySeparatorChar
    if (-not $candidate.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "$Description escapes the staged evidence root."
    }
    return (Get-RegularFile -LiteralPath $candidate -Description $Description).FullName
}

$archive = Get-RegularFile -LiteralPath $ArchivePath -Description 'protected release evidence archive'
$policy = Get-RegularFile -LiteralPath $PolicyPath -Description 'public release policy'
$actualArchiveSha256 = (Get-FileHash -LiteralPath $archive.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualArchiveSha256 -cne $ExpectedArchiveSha256.ToLowerInvariant()) {
    throw "Protected release evidence archive SHA-256 mismatch: expected $ExpectedArchiveSha256, found $actualArchiveSha256."
}

$outputFull = [System.IO.Path]::GetFullPath($OutputDirectory).TrimEnd('\', '/')
if ([string]::IsNullOrWhiteSpace($outputFull) -or [System.IO.Directory]::Exists($outputFull) -or [System.IO.File]::Exists($outputFull)) {
    throw "Protected release evidence output must be a fresh path: '$outputFull'."
}
$outputParent = [System.IO.Directory]::GetParent($outputFull)
if ($null -eq $outputParent -or -not $outputParent.Exists) {
    throw "Protected release evidence output parent must already exist: '$outputFull'."
}
Assert-NoReparseDirectoryChain -LiteralPath $outputParent.FullName
[void] [System.IO.Directory]::CreateDirectory($outputFull)
$outputItem = Get-Item -LiteralPath $outputFull -Force
if (($outputItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw 'Protected release evidence output became a reparse point.'
}

Add-Type -AssemblyName System.IO.Compression
$archiveStream = [System.IO.File]::Open($archive.FullName, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::Read)
$zip = $null
$seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
$fileCount = 0L
$totalUncompressedBytes = 0L
try {
    $zip = [System.IO.Compression.ZipArchive]::new($archiveStream, [System.IO.Compression.ZipArchiveMode]::Read, $false)
    if ($zip.Entries.Count -gt $MaxEvidenceEntryCount) {
        throw "Protected release evidence archive exceeds $MaxEvidenceEntryCount entries."
    }
    foreach ($entry in $zip.Entries) {
        $isDirectory = $entry.FullName.EndsWith('/', [System.StringComparison]::Ordinal)
        $segments = Get-SafeRelativeSegments -RelativePath $entry.FullName -Description 'evidence archive entry' -AllowTrailingSlash:$isDirectory
        $portable = $segments -join '/'
        if (-not $seen.Add($portable)) {
            throw "Protected release evidence archive contains a duplicate path: '$portable'."
        }
        $unixType = (($entry.ExternalAttributes -shr 16) -band 0xF000)
        $windowsAttributes = ($entry.ExternalAttributes -band 0xFFFF)
        if ($unixType -eq 0xA000 -or ($windowsAttributes -band [int] [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Protected release evidence archive contains a link or reparse entry: '$portable'."
        }
        if ($entry.Length -lt 0) {
            throw "Protected release evidence archive contains an invalid entry length: '$portable'."
        }
        $totalUncompressedBytes += [long] $entry.Length
        if ($totalUncompressedBytes -gt $MaxEvidenceUncompressedBytes) {
            throw "Protected release evidence archive exceeds $MaxEvidenceUncompressedBytes uncompressed bytes."
        }

        $destination = [System.IO.Path]::GetFullPath((Join-Path $outputFull ($segments -join [System.IO.Path]::DirectorySeparatorChar)))
        $outputPrefix = $outputFull + [System.IO.Path]::DirectorySeparatorChar
        if (-not $destination.StartsWith($outputPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Protected release evidence archive entry escapes the output root: '$portable'."
        }
        if ($isDirectory) {
            [void] [System.IO.Directory]::CreateDirectory($destination)
            Assert-NoReparseDirectoryChain -LiteralPath $destination
            continue
        }

        $destinationParent = [System.IO.Directory]::GetParent($destination)
        [void] [System.IO.Directory]::CreateDirectory($destinationParent.FullName)
        Assert-NoReparseDirectoryChain -LiteralPath $destinationParent.FullName
        $input = $entry.Open()
        $output = [System.IO.File]::Open($destination, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write, [System.IO.FileShare]::None)
        try { $input.CopyTo($output) } finally { $output.Dispose(); $input.Dispose() }
        $fileCount += 1
    }
}
finally {
    if ($null -ne $zip) { $zip.Dispose() }
    $archiveStream.Dispose()
}

$policyJson = Get-Content -Raw -LiteralPath $policy.FullName -Encoding UTF8 | ConvertFrom-Json -Depth 100
$manifestReports = [System.Collections.Generic.List[object]]::new()
foreach ($sectionName in @('cleanVmValidation', 'dataSafety')) {
    $section = $policyJson.$sectionName
    if ([string] $section.evidenceSource -cne 'protected-external-archive') {
        throw "$sectionName must use protected-external-archive."
    }
    foreach ($externalOnlyProperty in @('sourceCommit', 'approved', 'approvalReference')) {
        if ($null -ne $section.PSObject.Properties[$externalOnlyProperty]) {
            throw "$sectionName policy must not embed '$externalOnlyProperty'; approval and source-commit binding belong to the protected external evidence manifest."
        }
    }
    $manifestPath = Resolve-StagedEvidenceFile `
        -Root $outputFull `
        -RelativePath ([string] $section.evidenceManifest) `
        -Description "$sectionName evidence manifest"
    $manifest = Get-Content -Raw -LiteralPath $manifestPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
    if ([long] $manifest.schemaVersion -ne 1 -or
        [string] $manifest.sourceCommit -cne $ExpectedSourceCommit.ToLowerInvariant()) {
        throw "$sectionName evidence manifest is not bound to the approved source commit."
    }
    $approvedProperty = $manifest.PSObject.Properties['approved']
    $approvalReferenceProperty = $manifest.PSObject.Properties['approvalReference']
    if ($null -eq $approvedProperty -or
        $approvedProperty.Value -isnot [bool] -or
        $approvedProperty.Value -ne $true -or
        $null -eq $approvalReferenceProperty -or
        $approvalReferenceProperty.Value -isnot [string] -or
        [string]::IsNullOrWhiteSpace([string] $approvalReferenceProperty.Value)) {
        throw "$sectionName evidence manifest is missing its external approval attestation."
    }
    $manifestReports.Add([ordered]@{
            section = $sectionName
            relativePath = [string] $section.evidenceManifest
            sha256 = (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
        })
}

$report = [ordered]@{
    schemaVersion = 1
    status = 'staged'
    source = 'protected-external-archive'
    sourceCommit = $ExpectedSourceCommit.ToLowerInvariant()
    archive = [ordered]@{
        fileName = $archive.Name
        sizeBytes = $archive.Length
        sha256 = $actualArchiveSha256
    }
    extracted = [ordered]@{
        fileCount = $fileCount
        totalUncompressedBytes = $totalUncompressedBytes
    }
    policySha256 = (Get-FileHash -LiteralPath $policy.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    manifests = $manifestReports.ToArray()
}
$json = $report | ConvertTo-Json -Depth 8
if (-not [string]::IsNullOrWhiteSpace($ReportPath)) {
    $reportFull = [System.IO.Path]::GetFullPath($ReportPath)
    if ([System.IO.File]::Exists($reportFull)) {
        throw "Evidence staging report already exists: '$reportFull'."
    }
    $reportParent = [System.IO.Directory]::GetParent($reportFull)
    if ($null -eq $reportParent -or -not $reportParent.Exists) {
        throw "Evidence staging report parent does not exist: '$reportFull'."
    }
    [System.IO.File]::WriteAllText($reportFull, "$json`n", [System.Text.UTF8Encoding]::new($false))
}
$json
