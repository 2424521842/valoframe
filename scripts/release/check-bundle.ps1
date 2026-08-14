<#
.SYNOPSIS
Performs fail-closed static checks on a Windows release bundle.

.DESCRIPTION
Validates the NSIS installer, external UNK staging executable, bundled FFmpeg,
FFmpeg provenance manifest, bundled license files, hashes, and Authenticode
signatures. In the default public gate the external UNK staging executable must
be NotSigned, while the embedded NSS executable and installer must both have
Valid signatures; FFmpeg source-compliance metadata must be release-ready.

Use -AllowUnsignedInternalRc only for an explicitly internal release candidate.
That switch permits NotSigned application artifacts and an unfinished public
FFmpeg redistribution record; it does not relax file, hash, path, or license
checks. HashMismatch and all other invalid signature states still fail.

Use -AllowUnsignedCommunityBeta only for the separately documented GitHub
prerelease channel. That profile still requires the exact minimal FFmpeg binary
archive and corresponding-source archive beside the installer, validates the
channel decision record, and keeps the strict public-release policy blocked.

Use -AllowPersonalCommunityStable only for the repository-owner-authorized,
free personal community stable channel. That profile requires the exact minimal
FFmpeg binary and corresponding-source archives beside the installer, validates
their stable-tag binding and channel decision, and does not claim strict public
release approval. Valid Authenticode signatures are accepted when present, but
NotSigned application artifacts are also allowed and every other signature
status remains a hard failure.

.PARAMETER MainExecutablePath
Path to the post-build external UNK staging executable used for byte provenance.
The actually shipped NSS variant is extracted from the installer and checked.

.PARAMETER NsisBundlePath
Path to the NSIS installer executable.

.PARAMETER NsisScriptPath
Path to the generated Tauri installer.nsi used to build the installer. Its
Install-section payload declarations are bound to the supplied main executable
and resource files.

.PARAMETER NsisExtractorPath
Path to a full 7-Zip 7z.exe with the NSIS archive handler. The installer is
listed and only the expected application/compliance payload files are extracted into a
fresh temporary directory for hash comparison; the installer is never run.

.PARAMETER VerifiedPayloadOutputDirectory
Optional pre-created empty directory below the caller's temporary directory.
After every payload check passes, the controlled-extraction files are moved
there for a subsequent startup smoke. Without this parameter no payload remains.

.PARAMETER ResourceDirectory
Path to the unpacked resource directory. It must contain bin\ffmpeg.exe and the
FFmpeg license directory.

.PARAMETER FfmpegManifestPath
Path to the pinned Windows x64 FFmpeg provenance manifest.

.PARAMETER ExpectedMainExecutableSha256
Optional expected SHA-256 for the main executable.

.PARAMETER ExpectedNsisBundleSha256
Optional expected SHA-256 for the NSIS installer.

.PARAMETER CommunityBetaFfmpegArchivePath
Path to the exact minimal FFmpeg binary archive that will be uploaded beside a
Community Beta installer.

.PARAMETER CommunityBetaSourceBundlePath
Path to the exact FFmpeg corresponding-source archive that will be uploaded
beside a Community Beta installer.

.PARAMETER PersonalCommunityStableDecisionPath
Path to the repository-owner decision for the personal community stable release.

.PARAMETER FFmpegArchivePath
Path to the exact minimal FFmpeg binary archive that will be uploaded beside a
personal community stable installer.

.PARAMETER SourceBundlePath
Path to the exact FFmpeg corresponding-source archive that will be uploaded
beside a personal community stable installer.

.EXAMPLE
pwsh ./scripts/release/check-bundle.ps1 `
  -MainExecutablePath ./artifacts/app/valorant-highlight-manager.exe `
  -NsisBundlePath ./artifacts/bundle/nsis/ValorantHighlightManager-setup.exe `
  -NsisScriptPath ./artifacts/nsis/x64/installer.nsi `
  -NsisExtractorPath 'C:\Program Files\7-Zip\7z.exe' `
  -ResourceDirectory ./artifacts/app

.EXAMPLE
pwsh ./scripts/release/check-bundle.ps1 `
  -MainExecutablePath ./artifacts/app/valorant-highlight-manager.exe `
  -NsisBundlePath ./artifacts/bundle/nsis/ValorantHighlightManager-setup.exe `
  -NsisScriptPath ./artifacts/nsis/x64/installer.nsi `
  -NsisExtractorPath 'C:\Program Files\7-Zip\7z.exe' `
  -ResourceDirectory ./artifacts/app `
  -AllowUnsignedInternalRc
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $MainExecutablePath,

    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $NsisBundlePath,

    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $NsisScriptPath,

    [ValidateNotNullOrEmpty()]
    [string] $NsisExtractorPath,

    [ValidateNotNullOrEmpty()]
    [string] $VerifiedPayloadOutputDirectory,

    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ResourceDirectory,

    [ValidateNotNullOrEmpty()]
    [string] $FfmpegManifestPath = (Join-Path $PSScriptRoot '..\..\third_party\ffmpeg\windows-x64.json'),

    [ValidateNotNullOrEmpty()]
    [string] $FfmpegRelativePath = 'bin\ffmpeg.exe',

    [ValidateNotNullOrEmpty()]
    [string[]] $LicenseRelativePaths = @(
        'licenses\ffmpeg\COPYING.LGPLv3.txt',
        'licenses\ffmpeg\COPYING.GPLv3.txt',
        'licenses\ffmpeg\BUILD-INFO.json',
        'licenses\ffmpeg\SOURCE-OFFER.md'
    ),

    [ValidateNotNullOrEmpty()]
    [string] $ThirdPartyComplianceRelativeRoot = 'licenses\third-party',

    [ValidateNotNullOrEmpty()]
    [string] $PublicReleasePolicyPath = (Join-Path $PSScriptRoot '..\..\release\public-release-policy.json'),

    [string] $SigntoolPath,

    [string] $ExpectedMainExecutableSha256,

    [string] $ExpectedNsisBundleSha256,

    [switch] $AllowUnsignedInternalRc,

    [switch] $AllowUnsignedCommunityBeta,

    [switch] $AllowPersonalCommunityStable,

    [string] $CommunityBetaFfmpegArchivePath,

    [string] $CommunityBetaSourceBundlePath,

    [ValidateNotNullOrEmpty()]
    [string] $CommunityBetaDecisionPath = (Join-Path $PSScriptRoot '..\..\release\approvals\community-beta-v0.1.0.json'),

    [string] $PersonalCommunityStableDecisionPath,

    [string] $FFmpegArchivePath,

    [string] $SourceBundlePath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($AllowUnsignedInternalRc -and $AllowUnsignedCommunityBeta) {
    throw '-AllowUnsignedInternalRc and -AllowUnsignedCommunityBeta are mutually exclusive.'
}
if ($AllowPersonalCommunityStable -and ($AllowUnsignedInternalRc -or $AllowUnsignedCommunityBeta)) {
    throw '-AllowPersonalCommunityStable is mutually exclusive with -AllowUnsignedInternalRc and -AllowUnsignedCommunityBeta.'
}

function Test-IsWindows {
    return [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
        [System.Runtime.InteropServices.OSPlatform]::Windows
    )
}

function Get-CanonicalExistingPath {
    param(
        [Parameter(Mandatory = $true)] [string] $LiteralPath,
        [Parameter(Mandatory = $true)] [bool] $RequireDirectory,
        [Parameter(Mandatory = $true)] [string] $Description
    )

    $resolvedPaths = @(Resolve-Path -LiteralPath $LiteralPath -ErrorAction Stop)
    if ($resolvedPaths.Count -ne 1) {
        throw "$Description must resolve to exactly one path: '$LiteralPath'."
    }

    $resolved = $resolvedPaths[0]
    $providerPath = $resolved.ProviderPath
    $item = Get-Item -LiteralPath $providerPath -Force -ErrorAction Stop
    if ($RequireDirectory -and -not $item.PSIsContainer) {
        throw "$Description must be a directory: '$providerPath'."
    }
    if (-not $RequireDirectory -and $item.PSIsContainer) {
        throw "$Description must be a regular file: '$providerPath'."
    }
    if (-not $RequireDirectory -and $item.Length -le 0) {
        throw "$Description must not be empty: '$providerPath'."
    }

    return [System.IO.Path]::GetFullPath($providerPath)
}

function Test-PathWithinRoot {
    param(
        [Parameter(Mandatory = $true)] [string] $Root,
        [Parameter(Mandatory = $true)] [string] $Candidate,
        [switch] $AllowRoot
    )

    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    )
    $candidateFull = [System.IO.Path]::GetFullPath($Candidate).TrimEnd(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    )
    if ($AllowRoot -and [string]::Equals(
            $rootFull,
            $candidateFull,
            [System.StringComparison]::OrdinalIgnoreCase
        )) {
        return $true
    }

    $prefix = $rootFull + [System.IO.Path]::DirectorySeparatorChar
    return $candidateFull.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)
}

function Assert-NoReparsePoint {
    param(
        [Parameter(Mandatory = $true)] [string] $Root,
        [Parameter(Mandatory = $true)] [string] $Target,
        [Parameter(Mandatory = $true)] [string] $Description
    )

    if (-not (Test-PathWithinRoot -Root $Root -Candidate $Target -AllowRoot)) {
        throw "$Description escapes the resource directory: '$Target'."
    }

    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    $cursor = [System.IO.Path]::GetFullPath($Target).TrimEnd('\', '/')
    while ($true) {
        $item = Get-Item -LiteralPath $cursor -Force -ErrorAction Stop
        if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "$Description traverses a reparse point: '$cursor'."
        }
        if ([string]::Equals($cursor, $rootFull, [System.StringComparison]::OrdinalIgnoreCase)) {
            break
        }

        $parent = [System.IO.Directory]::GetParent($cursor)
        if ($null -eq $parent) {
            throw "$Description could not be proven to remain below '$rootFull'."
        }
        $cursor = $parent.FullName.TrimEnd('\', '/')
    }
}

function Get-ResourceFile {
    param(
        [Parameter(Mandatory = $true)] [string] $Root,
        [Parameter(Mandatory = $true)] [string] $RelativePath,
        [Parameter(Mandatory = $true)] [string] $Description
    )

    if ([System.IO.Path]::IsPathRooted($RelativePath)) {
        throw "$Description must be relative to the resource directory: '$RelativePath'."
    }

    $candidate = [System.IO.Path]::GetFullPath((Join-Path $Root $RelativePath))
    if (-not (Test-PathWithinRoot -Root $Root -Candidate $candidate)) {
        throw "$Description escapes the resource directory: '$RelativePath'."
    }

    $resolved = Get-CanonicalExistingPath -LiteralPath $candidate -RequireDirectory $false -Description $Description
    Assert-NoReparsePoint -Root $Root -Target $resolved -Description $Description
    return $resolved
}

function Get-ResourceDirectory {
    param(
        [Parameter(Mandatory = $true)] [string] $Root,
        [Parameter(Mandatory = $true)] [string] $RelativePath,
        [Parameter(Mandatory = $true)] [string] $Description
    )

    if ([System.IO.Path]::IsPathRooted($RelativePath)) {
        throw "$Description must be relative to the resource directory: '$RelativePath'."
    }

    $candidate = [System.IO.Path]::GetFullPath((Join-Path $Root $RelativePath))
    if (-not (Test-PathWithinRoot -Root $Root -Candidate $candidate)) {
        throw "$Description escapes the resource directory: '$RelativePath'."
    }

    $resolved = Get-CanonicalExistingPath -LiteralPath $candidate -RequireDirectory $true -Description $Description
    Assert-NoReparsePoint -Root $Root -Target $resolved -Description $Description
    return $resolved
}

function Get-Sha256 {
    param([Parameter(Mandatory = $true)] [string] $LiteralPath)

    return (Get-FileHash -LiteralPath $LiteralPath -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()
}

function Assert-Sha256Text {
    param(
        [Parameter(Mandatory = $true)] [string] $Value,
        [Parameter(Mandatory = $true)] [string] $Description
    )

    if ($Value -notmatch '^[0-9a-fA-F]{64}$') {
        throw "$Description must be a 64-character hexadecimal SHA-256 value."
    }
}

function Assert-ExpectedHash {
    param(
        [Parameter(Mandatory = $true)] [string] $Actual,
        [AllowEmptyString()] [string] $Expected,
        [Parameter(Mandatory = $true)] [string] $Description
    )

    if ([string]::IsNullOrWhiteSpace($Expected)) {
        return
    }
    Assert-Sha256Text -Value $Expected -Description "$Description expected hash"
    if (-not [string]::Equals($Actual, $Expected, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "$Description SHA-256 mismatch. Expected '$Expected', got '$Actual'."
    }
}

function Assert-PeFile {
    param(
        [Parameter(Mandatory = $true)] [string] $LiteralPath,
        [Parameter(Mandatory = $true)] [string] $Description
    )

    $stream = [System.IO.File]::Open(
        $LiteralPath,
        [System.IO.FileMode]::Open,
        [System.IO.FileAccess]::Read,
        [System.IO.FileShare]::Read
    )
    try {
        if ($stream.Length -lt 64) {
            throw "$Description is too small to be a Windows PE file: '$LiteralPath'."
        }
        if ($stream.ReadByte() -ne 0x4d -or $stream.ReadByte() -ne 0x5a) {
            throw "$Description does not have an MZ header: '$LiteralPath'."
        }

        $stream.Position = 0x3c
        $offsetBytes = [byte[]]::new(4)
        if ($stream.Read($offsetBytes, 0, 4) -ne 4) {
            throw "$Description has a truncated PE header offset."
        }
        $peOffset = [System.BitConverter]::ToUInt32($offsetBytes, 0)
        if ($peOffset -gt ($stream.Length - 4)) {
            throw "$Description has an invalid PE header offset."
        }
        $stream.Position = $peOffset
        $signature = [byte[]]::new(4)
        if ($stream.Read($signature, 0, 4) -ne 4 -or
            $signature[0] -ne 0x50 -or $signature[1] -ne 0x45 -or
            $signature[2] -ne 0 -or $signature[3] -ne 0) {
            throw "$Description does not have a valid PE signature."
        }
    }
    finally {
        $stream.Dispose()
    }
}

function Get-PeLayoutReport {
    param(
        [Parameter(Mandatory = $true)] [string] $LiteralPath,
        [Parameter(Mandatory = $true)] [string] $Description
    )

    $stream = [System.IO.File]::Open(
        $LiteralPath,
        [System.IO.FileMode]::Open,
        [System.IO.FileAccess]::Read,
        [System.IO.FileShare]::Read
    )
    $reader = [System.IO.BinaryReader]::new($stream, [System.Text.Encoding]::ASCII, $true)
    try {
        if ($stream.Length -lt 64 -or $reader.ReadUInt16() -ne 0x5a4d) {
            throw "$Description is not a valid PE image."
        }
        $stream.Position = 0x3c
        $peOffset = [long] $reader.ReadUInt32()
        if ($peOffset -lt 64 -or $peOffset -gt ($stream.Length - 24)) {
            throw "$Description has an invalid PE header offset."
        }
        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550) {
            throw "$Description is missing the PE signature."
        }
        $machine = $reader.ReadUInt16()
        $sectionCount = [int] $reader.ReadUInt16()
        if ($sectionCount -le 0 -or $sectionCount -gt 96) {
            throw "$Description has an unsafe PE section count: $sectionCount."
        }
        $stream.Position = $peOffset + 20
        $optionalHeaderSize = [int] $reader.ReadUInt16()
        [void] $reader.ReadUInt16()
        $optionalStart = $peOffset + 24
        if ($optionalHeaderSize -lt 96 -or $optionalStart + $optionalHeaderSize -gt $stream.Length) {
            throw "$Description has an invalid PE optional header."
        }

        $stream.Position = $optionalStart
        $optionalMagic = $reader.ReadUInt16()
        $dataDirectoryOffset = switch ($optionalMagic) {
            0x10b { $optionalStart + 96 }
            0x20b { $optionalStart + 112 }
            default { throw "$Description has an unsupported PE optional-header magic: 0x$($optionalMagic.ToString('x'))." }
        }
        $numberOfDirectoriesOffset = if ($optionalMagic -eq 0x10b) { $optionalStart + 92 } else { $optionalStart + 108 }
        $securityDirectoryOffset = $dataDirectoryOffset + (4 * 8)
        if ($securityDirectoryOffset + 8 -gt $optionalStart + $optionalHeaderSize) {
            throw "$Description PE optional header does not contain the security directory."
        }

        $stream.Position = $optionalStart + 60
        $sizeOfHeaders = [long] $reader.ReadUInt32()
        $stream.Position = $numberOfDirectoriesOffset
        $directoryCount = [long] $reader.ReadUInt32()
        [long] $certificateOffset = 0
        [long] $certificateSize = 0
        if ($directoryCount -gt 4) {
            $stream.Position = $securityDirectoryOffset
            $certificateOffset = [long] $reader.ReadUInt32()
            $certificateSize = [long] $reader.ReadUInt32()
        }
        if (($certificateOffset -eq 0) -xor ($certificateSize -eq 0)) {
            throw "$Description has an incomplete Authenticode certificate-table declaration."
        }

        $sectionTableOffset = $optionalStart + $optionalHeaderSize
        if ($sectionTableOffset + (40L * $sectionCount) -gt $stream.Length) {
            throw "$Description has a truncated PE section table."
        }
        [long] $overlayOffset = $sizeOfHeaders
        for ($index = 0; $index -lt $sectionCount; $index++) {
            $stream.Position = $sectionTableOffset + (40L * $index) + 16
            $rawSize = [long] $reader.ReadUInt32()
            $rawOffset = [long] $reader.ReadUInt32()
            if ($rawSize -gt 0) {
                $rawEnd = $rawOffset + $rawSize
                if ($rawOffset -le 0 -or $rawEnd -gt $stream.Length) {
                    throw "$Description has a PE section outside the file."
                }
                if ($rawEnd -gt $overlayOffset) {
                    $overlayOffset = $rawEnd
                }
            }
        }
        if ($overlayOffset -le 0 -or $overlayOffset -gt $stream.Length) {
            throw "$Description has an invalid PE overlay offset."
        }
        if ($certificateOffset -gt 0) {
            if (($certificateOffset % 8) -ne 0 -or $certificateOffset -lt $overlayOffset -or
                $certificateOffset + $certificateSize -ne $stream.Length) {
                throw "$Description has an invalid or trailing Authenticode certificate table."
            }
        }

        return [ordered]@{
            fileSize = $stream.Length
            machine = $machine
            sectionCount = $sectionCount
            overlayOffset = $overlayOffset
            certificateOffset = $certificateOffset
            certificateSize = $certificateSize
            checksumOffset = $optionalStart + 64
            securityDirectoryOffset = $securityDirectoryOffset
        }
    }
    finally {
        $reader.Dispose()
        $stream.Dispose()
    }
}

function Get-NsisHeaderReport {
    param(
        [Parameter(Mandatory = $true)] [string] $LiteralPath,
        [Parameter(Mandatory = $true)] [object] $PeLayout
    )

    $overlayOffset = [long] $PeLayout.overlayOffset
    $stream = [System.IO.File]::Open(
        $LiteralPath,
        [System.IO.FileMode]::Open,
        [System.IO.FileAccess]::Read,
        [System.IO.FileShare]::Read
    )
    $reader = [System.IO.BinaryReader]::new($stream, [System.Text.Encoding]::ASCII, $true)
    try {
        if ($stream.Length - $overlayOffset -lt 28) {
            throw 'NSIS installer has no complete first header at the PE overlay boundary.'
        }
        $stream.Position = $overlayOffset
        $flags = $reader.ReadUInt32()
        $signature = $reader.ReadUInt32()
        $magic = [System.Text.Encoding]::ASCII.GetString($reader.ReadBytes(12))
        $headerLength = [long] $reader.ReadUInt32()
        $followingDataLength = [long] $reader.ReadUInt32()
        if (($flags -band 0xfffffff0L) -ne 0) {
            throw "NSIS first header contains unsupported flags: 0x$($flags.ToString('x8'))."
        }
        if ($signature -ne 0xdeadbeefL -or $magic -cne 'NullsoftInst') {
            throw 'PE overlay is not an NSIS first header (DEADBEEF/NullsoftInst mismatch).'
        }
        if ($headerLength -lt 28 -or $headerLength -gt $followingDataLength) {
            throw 'NSIS first header declares an invalid header length.'
        }

        $nsisEnd = $overlayOffset + $followingDataLength
        $certificateOffset = [long] $PeLayout.certificateOffset
        if ($certificateOffset -eq 0) {
            if ($nsisEnd -ne $stream.Length) {
                throw 'Unsigned NSIS payload length does not cover the complete PE overlay.'
            }
        }
        else {
            if ($nsisEnd -gt $certificateOffset -or $certificateOffset - $nsisEnd -gt 7) {
                throw 'Signed NSIS payload does not terminate immediately before the Authenticode table.'
            }
            $stream.Position = $nsisEnd
            while ($stream.Position -lt $certificateOffset) {
                if ($stream.ReadByte() -ne 0) {
                    throw 'Signed NSIS payload has non-zero data before the Authenticode table.'
                }
            }
        }

        return [ordered]@{
            flags = $flags
            magic = $magic
            headerLength = $headerLength
            followingDataLength = $followingDataLength
            overlayOffset = $overlayOffset
            payloadEndOffset = $nsisEnd
            authenticodeBytes = [long] $PeLayout.certificateSize
        }
    }
    finally {
        $reader.Dispose()
        $stream.Dispose()
    }
}

function Get-RequiredJsonProperty {
    param(
        [Parameter(Mandatory = $true)] [object] $Object,
        [Parameter(Mandatory = $true)] [string] $Name,
        [Parameter(Mandatory = $true)] [string] $Context
    )

    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property -or $null -eq $property.Value) {
        throw "$Context is missing required property '$Name'."
    }
    return $property.Value
}

function Get-RequiredJsonString {
    param(
        [Parameter(Mandatory = $true)] [object] $Object,
        [Parameter(Mandatory = $true)] [string] $Name,
        [Parameter(Mandatory = $true)] [string] $Context
    )

    $value = Get-RequiredJsonProperty -Object $Object -Name $Name -Context $Context
    if ($value -isnot [string] -or [string]::IsNullOrWhiteSpace($value)) {
        throw "$Context property '$Name' must be a non-empty string."
    }
    return $value
}

function Assert-HttpsUrl {
    param(
        [Parameter(Mandatory = $true)] [string] $Value,
        [Parameter(Mandatory = $true)] [string] $Description
    )

    $uri = $null
    if (-not [System.Uri]::TryCreate($Value, [System.UriKind]::Absolute, [ref] $uri) -or
        $uri.Scheme -ne [System.Uri]::UriSchemeHttps) {
        throw "$Description must be an absolute HTTPS URL."
    }
}

function Get-SignatureReport {
    param(
        [Parameter(Mandatory = $true)] [string] $LiteralPath,
        [Parameter(Mandatory = $true)] [string] $Description,
        [Parameter(Mandatory = $true)] [bool] $PermitUnsigned,
        [AllowEmptyString()] [string] $UnsignedAcceptanceReason,
        [AllowNull()] [object] $SigningRequirements,
        [switch] $HashPinnedOnly
    )

    $signature = Get-AuthenticodeSignature -LiteralPath $LiteralPath -ErrorAction Stop
    $status = [string] $signature.Status
    if ($status -eq 'HashMismatch') {
        throw "$Description has an Authenticode HashMismatch status."
    }

    if (-not $HashPinnedOnly) {
        if ($status -eq 'NotSigned' -and $PermitUnsigned) {
            $reason = if ([string]::IsNullOrWhiteSpace($UnsignedAcceptanceReason)) {
                '-AllowUnsignedInternalRc was supplied'
            }
            else {
                $UnsignedAcceptanceReason
            }
            Write-Warning "$Description is unsigned; accepted because $reason."
        }
        elseif ($status -ne 'Valid') {
            throw "$Description Authenticode status must be Valid$(if ($PermitUnsigned) { ' or NotSigned' } else { '' }); got '$status'."
        }
    }

    $certificate = $signature.SignerCertificate
    $timestampCertificate = $signature.TimeStamperCertificate
    $signtoolReport = $null
    if ($null -ne $SigningRequirements) {
        if ($status -cne 'Valid' -or $null -eq $certificate) {
            throw "$Description must have a valid signer certificate before publisher binding is checked."
        }
        if (-not [string]::Equals([string] $certificate.Subject, [string] $SigningRequirements.expectedPublisherSubject, [System.StringComparison]::Ordinal)) {
            throw "$Description signer subject does not match the approved publisher subject."
        }
        if (-not [string]::Equals(([string] $certificate.Thumbprint).Replace(' ', ''), [string] $SigningRequirements.expectedCertificateThumbprint, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "$Description signer thumbprint does not match the approved certificate."
        }
        if ($null -eq $timestampCertificate) {
            throw "$Description does not contain a trusted Authenticode timestamp certificate."
        }
        $signtoolResult = Invoke-CheckedExternalProcess `
            -Executable ([string] $SigningRequirements.signtoolPath) `
            -Arguments @('verify', '/pa', '/all', '/v', $LiteralPath) `
            -TimeoutSeconds 60 `
            -Description "$Description signtool chain verification"
        $signtoolReport = [ordered]@{
            path = [string] $SigningRequirements.signtoolPath
            sha256 = [string] $SigningRequirements.signtoolSha256
            exitCode = [long] $signtoolResult.exitCode
            verification = 'signtool verify /pa /all /v'
            output = [string] $signtoolResult.combined
        }
    }
    return [ordered]@{
        status = $status
        statusMessage = [string] $signature.StatusMessage
        signerSubject = if ($null -eq $certificate) { $null } else { $certificate.Subject }
        signerThumbprint = if ($null -eq $certificate) { $null } else { $certificate.Thumbprint }
        signerNotAfterUtc = if ($null -eq $certificate) { $null } else { $certificate.NotAfter.ToUniversalTime().ToString('o') }
        timestampSubject = if ($null -eq $timestampCertificate) { $null } else { $timestampCertificate.Subject }
        timestampThumbprint = if ($null -eq $timestampCertificate) { $null } else { $timestampCertificate.Thumbprint }
        timestampNotAfterUtc = if ($null -eq $timestampCertificate) { $null } else { $timestampCertificate.NotAfter.ToUniversalTime().ToString('o') }
        signtool = $signtoolReport
    }
}

function Invoke-CheckedExternalProcess {
    param(
        [Parameter(Mandatory = $true)] [string] $Executable,
        [Parameter(Mandatory = $true)] [string[]] $Arguments,
        [Parameter(Mandatory = $true)] [int] $TimeoutSeconds,
        [Parameter(Mandatory = $true)] [string] $Description
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
    foreach ($argument in $Arguments) {
        [void] $startInfo.ArgumentList.Add($argument)
    }

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) {
            throw "Could not start $Description."
        }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            try { $process.Kill($true) } catch { Write-Verbose $_.Exception.Message }
            $process.WaitForExit()
            throw "$Description exceeded the $TimeoutSeconds second timeout."
        }
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0) {
            throw "$Description failed with exit code $($process.ExitCode).`nSTDOUT:`n$stdout`nSTDERR:`n$stderr"
        }
        return [ordered]@{
            exitCode = 0
            stdout = $stdout
            stderr = $stderr
            combined = $stdout + "`n" + $stderr
        }
    }
    finally {
        $process.Dispose()
    }
}

function Resolve-NsisExtractor {
    param([AllowEmptyString()] [string] $ConfiguredPath)

    if (-not [string]::IsNullOrWhiteSpace($ConfiguredPath)) {
        return Get-CanonicalExistingPath -LiteralPath $ConfiguredPath -RequireDirectory $false -Description 'NSIS extractor'
    }

    $command = Get-Command 7z.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -ne $command) {
        return Get-CanonicalExistingPath -LiteralPath $command.Source -RequireDirectory $false -Description 'NSIS extractor'
    }
    $fallback = Join-Path $env:ProgramFiles '7-Zip\7z.exe'
    if ([System.IO.File]::Exists($fallback)) {
        return Get-CanonicalExistingPath -LiteralPath $fallback -RequireDirectory $false -Description 'NSIS extractor'
    }
    throw 'A full 7-Zip 7z.exe with the NSIS handler is required. Pass -NsisExtractorPath explicitly.'
}

function Resolve-Signtool {
    param([AllowEmptyString()] [string] $ConfiguredPath)

    if (-not [string]::IsNullOrWhiteSpace($ConfiguredPath)) {
        $resolved = Get-CanonicalExistingPath -LiteralPath $ConfiguredPath -RequireDirectory $false -Description 'signtool'
    }
    else {
        $commands = @(Get-Command signtool.exe -CommandType Application -All -ErrorAction SilentlyContinue)
        if ($commands.Count -ne 1) {
            throw "Public release verification requires exactly one signtool.exe on PATH, or an explicit -SigntoolPath; found $($commands.Count)."
        }
        $resolved = Get-CanonicalExistingPath -LiteralPath $commands[0].Source -RequireDirectory $false -Description 'signtool'
    }
    if ([System.IO.Path]::GetFileName($resolved) -cne 'signtool.exe') {
        throw "Public release verification requires Microsoft's signtool.exe: '$resolved'."
    }
    $item = Get-Item -LiteralPath $resolved -Force
    if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "signtool.exe must not be a reparse point: '$resolved'."
    }
    Assert-PeFile -LiteralPath $resolved -Description 'signtool'
    $signature = Get-AuthenticodeSignature -LiteralPath $resolved -ErrorAction Stop
    if ([string] $signature.Status -cne 'Valid' -or $null -eq $signature.SignerCertificate -or
        [string] $signature.SignerCertificate.Subject -notmatch '(?i)(^|,\s*)CN=Microsoft (Corporation|Windows)($|,)') {
        throw 'signtool.exe must have a valid Microsoft Authenticode signature.'
    }
    return [ordered]@{
        path = $resolved
        sha256 = Get-Sha256 -LiteralPath $resolved
        signerSubject = [string] $signature.SignerCertificate.Subject
        signerThumbprint = [string] $signature.SignerCertificate.Thumbprint
    }
}

function Get-PublicSigningRequirements {
    param(
        [Parameter(Mandatory = $true)] [string] $RepositoryRoot,
        [Parameter(Mandatory = $true)] [string] $PolicyPath,
        [AllowEmptyString()] [string] $ConfiguredSigntoolPath
    )

    $policyFile = Get-CanonicalExistingPath -LiteralPath $PolicyPath -RequireDirectory $false -Description 'public release policy'
    if (-not (Test-PathWithinRoot -Root $RepositoryRoot -Candidate $policyFile)) {
        throw "Public release policy must be inside the repository root: '$policyFile'."
    }
    $policy = Get-Content -Raw -LiteralPath $policyFile -Encoding UTF8 | ConvertFrom-Json -Depth 100
    if ([long] (Get-RequiredJsonProperty -Object $policy -Name 'schemaVersion' -Context 'public release policy') -ne 1 -or
        [string] (Get-RequiredJsonProperty -Object $policy -Name 'releaseMode' -Context 'public release policy') -cne 'public') {
        throw 'Unsupported public release policy schema or mode.'
    }
    $identity = Get-RequiredJsonProperty -Object $policy -Name 'identity' -Context 'public release policy'
    $signing = Get-RequiredJsonProperty -Object $policy -Name 'authenticode' -Context 'public release policy'
    foreach ($requirement in @(
            [ordered]@{ object = $identity; name = 'publisherApproved' },
            [ordered]@{ object = $signing; name = 'certificateProvisioned' },
            [ordered]@{ object = $signing; name = 'trustedTimestampRequired' },
            [ordered]@{ object = $signing; name = 'signtoolVerificationRequired' }
        )) {
        $value = Get-RequiredJsonProperty -Object $requirement.object -Name $requirement.name -Context 'public signing policy'
        if ($value -isnot [bool] -or -not [bool] $value) {
            throw "Public signing policy '$($requirement.name)' must be the Boolean true."
        }
    }
    $publisherSubject = Get-RequiredJsonString -Object $identity -Name 'publisherSubject' -Context 'public release identity'
    $expectedSubject = Get-RequiredJsonString -Object $signing -Name 'expectedPublisherSubject' -Context 'public signing policy'
    if (-not [string]::Equals($publisherSubject, $expectedSubject, [System.StringComparison]::Ordinal)) {
        throw 'Public signing policy expectedPublisherSubject must exactly match identity.publisherSubject.'
    }
    [void] (Get-RequiredJsonString -Object $identity -Name 'publisherApprovalReference' -Context 'public release identity')
    [void] (Get-RequiredJsonString -Object $signing -Name 'approvalReference' -Context 'public signing policy')
    $thumbprint = (Get-RequiredJsonString -Object $signing -Name 'expectedCertificateThumbprint' -Context 'public signing policy').Replace(' ', '')
    if ($thumbprint -cnotmatch '^[0-9A-Fa-f]{40}$') {
        throw 'Public signing policy expectedCertificateThumbprint must be a 40-character certificate thumbprint.'
    }
    $timestampUrl = Get-RequiredJsonString -Object $signing -Name 'timestampUrl' -Context 'public signing policy'
    Assert-HttpsUrl -Value $timestampUrl -Description 'public signing timestampUrl'
    $signtool = Resolve-Signtool -ConfiguredPath $ConfiguredSigntoolPath
    return [ordered]@{
        policyPath = $policyFile
        policySha256 = Get-Sha256 -LiteralPath $policyFile
        expectedPublisherSubject = $expectedSubject
        expectedCertificateThumbprint = $thumbprint.ToUpperInvariant()
        timestampUrl = $timestampUrl
        signtoolPath = [string] $signtool.path
        signtoolSha256 = [string] $signtool.sha256
        signtoolSignerSubject = [string] $signtool.signerSubject
        signtoolSignerThumbprint = [string] $signtool.signerThumbprint
    }
}

function Resolve-VerifiedPayloadOutput {
    param([AllowEmptyString()] [string] $ConfiguredPath)

    if ([string]::IsNullOrWhiteSpace($ConfiguredPath)) {
        return $null
    }

    $output = Get-CanonicalExistingPath `
        -LiteralPath $ConfiguredPath `
        -RequireDirectory $true `
        -Description 'verified NSIS payload output directory'
    $outputItem = Get-Item -LiteralPath $output -Force
    if (($outputItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Verified NSIS payload output directory must not be a reparse point: '$output'."
    }
    if (@(Get-ChildItem -LiteralPath $output -Force).Count -ne 0) {
        throw "Verified NSIS payload output directory must be empty before validation: '$output'."
    }

    $candidateRoots = [System.Collections.Generic.List[string]]::new()
    if (-not [string]::IsNullOrWhiteSpace($env:RUNNER_TEMP) -and
        [System.IO.Directory]::Exists($env:RUNNER_TEMP)) {
        $candidateRoots.Add([System.IO.Path]::GetFullPath($env:RUNNER_TEMP))
    }
    $systemTemp = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd('\')
    if (-not $candidateRoots.Contains($systemTemp)) {
        $candidateRoots.Add($systemTemp)
    }

    $allowedRoot = $null
    foreach ($candidateRoot in $candidateRoots) {
        if (Test-PathWithinRoot -Root $candidateRoot -Candidate $output) {
            $allowedRoot = $candidateRoot
            break
        }
    }
    if ($null -eq $allowedRoot) {
        throw "Verified NSIS payload output directory must be below RUNNER_TEMP or the process temporary directory: '$output'."
    }
    $configuration = [ordered]@{
        path = $output
        allowedRoot = [System.IO.Path]::GetFullPath($allowedRoot).TrimEnd('\')
    }
    Assert-VerifiedPayloadOutputBoundary -Configuration $configuration -RequireEmpty
    return $configuration
}

function Assert-VerifiedPayloadOutputBoundary {
    param(
        [Parameter(Mandatory = $true)] [object] $Configuration,
        [switch] $RequireEmpty
    )

    $output = [System.IO.Path]::GetFullPath([string] $Configuration.path)
    $allowedRoot = [System.IO.Path]::GetFullPath([string] $Configuration.allowedRoot).TrimEnd('\')
    if (-not [System.IO.Directory]::Exists($allowedRoot) -or
        -not [System.IO.Directory]::Exists($output) -or
        -not (Test-PathWithinRoot -Root $allowedRoot -Candidate $output)) {
        throw 'Verified NSIS payload output/root boundary no longer exists or is outside the approved temporary root.'
    }
    $rootItem = Get-Item -LiteralPath $allowedRoot -Force
    if (-not $rootItem.PSIsContainer -or
        ($rootItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Verified NSIS payload allowed root must be a non-reparse directory: '$allowedRoot'."
    }
    Assert-NoReparsePoint -Root $allowedRoot -Target $output -Description 'verified NSIS payload output directory'
    if ($RequireEmpty -and @(Get-ChildItem -LiteralPath $output -Force).Count -ne 0) {
        throw "Verified NSIS payload output directory must still be empty: '$output'."
    }
}

function Assert-VerifiedPayloadMatchesReport {
    param(
        [Parameter(Mandatory = $true)] [object] $Configuration,
        [Parameter(Mandatory = $true)] [object[]] $EntryReports
    )

    Assert-VerifiedPayloadOutputBoundary -Configuration $Configuration
    $output = [System.IO.Path]::GetFullPath([string] $Configuration.path)
    $allItems = @(Get-ChildItem -LiteralPath $output -Recurse -Force)
    foreach ($item in $allItems) {
        if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Retained verified payload contains a reparse point: '$($item.FullName)'."
        }
    }
    $files = @($allItems | Where-Object { -not $_.PSIsContainer })
    if ($EntryReports.Count -le 0 -or $files.Count -ne $EntryReports.Count) {
        throw "Retained verified payload file count does not match the report; found $($files.Count), expected $($EntryReports.Count)."
    }

    $expectedFiles = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    foreach ($entry in $EntryReports) {
        $destination = ConvertTo-NsisPayloadPath -Value ([string] $entry.destination) -Description 'retained NSIS report destination'
        $relativeNative = $destination.Replace('\', [System.IO.Path]::DirectorySeparatorChar)
        $candidate = [System.IO.Path]::GetFullPath((Join-Path $output $relativeNative))
        if (-not (Test-PathWithinRoot -Root $output -Candidate $candidate) -or
            -not $expectedFiles.Add($candidate)) {
            throw "Retained NSIS report has an unsafe or duplicate destination: '$destination'."
        }
        $item = Get-Item -LiteralPath $candidate -Force -ErrorAction Stop
        if ($item.PSIsContainer -or
            ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0 -or
            $item.Length -ne [long] $entry.sizeBytes) {
            throw "Retained NSIS payload metadata mismatch for '$destination'."
        }
        $hash = Get-Sha256 -LiteralPath $candidate
        if (-not [string]::Equals($hash, [string] $entry.sha256, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Retained NSIS payload hash mismatch for '$destination'."
        }
    }
    foreach ($file in $files) {
        if (-not $expectedFiles.Contains([System.IO.Path]::GetFullPath($file.FullName))) {
            throw "Retained verified payload contains an unreported file: '$($file.FullName)'."
        }
    }

    return [ordered]@{
        outputDirectory = $output
        fileCount = $files.Count
        entriesMatchedReport = $true
        rootAndParentChainRecheckedAfterMove = $true
    }
}

function ConvertTo-NsisPayloadPath {
    param(
        [Parameter(Mandatory = $true)] [string] $Value,
        [Parameter(Mandatory = $true)] [string] $Description
    )

    if ([string]::IsNullOrWhiteSpace($Value) -or [System.IO.Path]::IsPathRooted($Value) -or
        $Value.Contains(':') -or $Value.Contains('/') -or $Value.IndexOf([char] 0) -ge 0) {
        throw "$Description is not a safe relative Windows payload path: '$Value'."
    }
    $segments = $Value.Split('\')
    foreach ($segment in $segments) {
        if ([string]::IsNullOrWhiteSpace($segment) -or $segment -eq '.' -or $segment -eq '..') {
            throw "$Description contains an unsafe path segment: '$Value'."
        }
    }
    return $segments -join '\'
}

function Find-ByteSequenceOffsets {
    param(
        [Parameter(Mandatory = $true)] [byte[]] $Data,
        [Parameter(Mandatory = $true)] [byte[]] $Pattern
    )

    if ($Pattern.Length -eq 0) {
        throw 'Byte-search pattern must not be empty.'
    }
    $offsets = [System.Collections.Generic.List[int]]::new()
    $searchFrom = 0
    while ($searchFrom -le $Data.Length - $Pattern.Length) {
        $candidate = [System.Array]::IndexOf[byte]($Data, $Pattern[0], $searchFrom)
        if ($candidate -lt 0 -or $candidate + $Pattern.Length -gt $Data.Length) {
            break
        }
        $matches = $true
        for ($index = 1; $index -lt $Pattern.Length; $index++) {
            if ($Data[$candidate + $index] -ne $Pattern[$index]) {
                $matches = $false
                break
            }
        }
        if ($matches) {
            $offsets.Add($candidate)
        }
        $searchFrom = $candidate + 1
    }
    return $offsets.ToArray()
}

function Get-ByteArraySha256 {
    param([Parameter(Mandatory = $true)] [byte[]] $Bytes)

    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        return [System.Convert]::ToHexString($sha256.ComputeHash($Bytes)).ToLowerInvariant()
    }
    finally {
        $sha256.Dispose()
    }
}

function Clear-CheckedByteRange {
    param(
        [Parameter(Mandatory = $true)] [byte[]] $Bytes,
        [Parameter(Mandatory = $true)] [long] $Offset,
        [Parameter(Mandatory = $true)] [int] $Length,
        [Parameter(Mandatory = $true)] [string] $Description
    )

    if ($Offset -lt 0 -or $Offset + $Length -gt $Bytes.Length) {
        throw "$Description is outside the comparable PE image."
    }
    [System.Array]::Clear($Bytes, [int] $Offset, $Length)
}

function Get-WinCertificateTableReport {
    param(
        [Parameter(Mandatory = $true)] [string] $LiteralPath,
        [Parameter(Mandatory = $true)] [object] $PeLayout
    )

    $tableOffset = [long] $PeLayout.certificateOffset
    $tableSize = [long] $PeLayout.certificateSize
    if ($tableOffset -le 0 -or $tableSize -lt 8 -or $tableOffset + $tableSize -ne [long] $PeLayout.fileSize) {
        throw 'Embedded main executable must have a non-empty WIN_CERTIFICATE table ending exactly at EOF.'
    }

    $stream = [System.IO.File]::Open(
        $LiteralPath,
        [System.IO.FileMode]::Open,
        [System.IO.FileAccess]::Read,
        [System.IO.FileShare]::Read
    )
    $reader = [System.IO.BinaryReader]::new($stream, [System.Text.Encoding]::ASCII, $true)
    $entryCount = 0
    try {
        $cursor = $tableOffset
        $tableEnd = $tableOffset + $tableSize
        while ($cursor -lt $tableEnd) {
            if ($tableEnd - $cursor -lt 8) {
                throw 'WIN_CERTIFICATE table has a truncated entry header.'
            }
            $stream.Position = $cursor
            $entryLength = [long] $reader.ReadUInt32()
            $revision = $reader.ReadUInt16()
            $certificateType = $reader.ReadUInt16()
            if ($entryLength -lt 8 -or $entryLength -gt ($tableEnd - $cursor)) {
                throw 'WIN_CERTIFICATE entry declares an invalid length.'
            }
            if ($revision -ne 0x0200 -or $certificateType -ne 0x0002) {
                throw "WIN_CERTIFICATE entry must be revision 2.0 PKCS signed data; got revision 0x$($revision.ToString('x4')), type 0x$($certificateType.ToString('x4'))."
            }
            $alignedLength = ($entryLength + 7L) -band -8L
            if ($cursor + $alignedLength -gt $tableEnd) {
                throw 'WIN_CERTIFICATE aligned entry exceeds the declared certificate table.'
            }
            $stream.Position = $cursor + $entryLength
            while ($stream.Position -lt $cursor + $alignedLength) {
                if ($stream.ReadByte() -ne 0) {
                    throw 'WIN_CERTIFICATE entry alignment padding must contain only zero bytes.'
                }
            }
            $cursor += $alignedLength
            $entryCount++
        }
        if ($cursor -ne $tableEnd -or $entryCount -eq 0) {
            throw 'WIN_CERTIFICATE entries do not exactly cover the declared certificate table.'
        }
    }
    finally {
        $reader.Dispose()
        $stream.Dispose()
    }

    return [ordered]@{
        offset = $tableOffset
        sizeBytes = $tableSize
        endOffset = $tableOffset + $tableSize
        entryCount = $entryCount
    }
}

function Get-TauriNsisMainPayloadReport {
    param(
        [Parameter(Mandatory = $true)] [string] $ExternalPath,
        [Parameter(Mandatory = $true)] [string] $EmbeddedPath,
        [Parameter(Mandatory = $true)] [bool] $AuthenticodeAware
    )

    $externalItem = Get-Item -LiteralPath $ExternalPath -Force
    $embeddedItem = Get-Item -LiteralPath $EmbeddedPath -Force
    if ($externalItem.Length -gt [int]::MaxValue -or $embeddedItem.Length -gt [int]::MaxValue) {
        throw 'Main executable is too large for bounded in-memory bundle comparison.'
    }
    $externalLayout = Get-PeLayoutReport -LiteralPath $ExternalPath -Description 'external main executable'
    $embeddedLayout = Get-PeLayoutReport -LiteralPath $EmbeddedPath -Description 'NSIS embedded main executable'
    $externalBytes = [System.IO.File]::ReadAllBytes($ExternalPath)
    $embeddedBytes = [System.IO.File]::ReadAllBytes($EmbeddedPath)

    $certificateReport = $null
    if ($AuthenticodeAware) {
        if ([long] $externalLayout.certificateOffset -ne 0 -or [long] $externalLayout.certificateSize -ne 0) {
            throw 'Public release external UNK staging executable must not contain an Authenticode certificate table.'
        }
        $alignedExternalLength = ($externalItem.Length + 7L) -band -8L
        if ([long] $embeddedLayout.certificateOffset -ne $alignedExternalLength) {
            throw 'Embedded NSS Authenticode certificate table must begin at Align8(external staging length).'
        }
        for ($offset = $externalItem.Length; $offset -lt $alignedExternalLength; $offset++) {
            if ($embeddedBytes[$offset] -ne 0) {
                throw 'Embedded NSS Authenticode alignment padding must contain only zero bytes.'
            }
        }
        $certificateReport = Get-WinCertificateTableReport -LiteralPath $EmbeddedPath -PeLayout $embeddedLayout
        foreach ($field in @('machine', 'sectionCount', 'overlayOffset', 'checksumOffset', 'securityDirectoryOffset')) {
            if ([long] $externalLayout.$field -ne [long] $embeddedLayout.$field) {
                throw "External and embedded main PE layouts disagree on '$field'."
            }
        }
        $embeddedComparableBytes = [byte[]]::new([int] $externalItem.Length)
        [System.Array]::Copy($embeddedBytes, 0, $embeddedComparableBytes, 0, $embeddedComparableBytes.Length)
    }
    else {
        if ($externalItem.Length -ne $embeddedItem.Length) {
            throw "NSIS main payload size differs from the external main executable. Expected $($externalItem.Length), got $($embeddedItem.Length)."
        }
        $embeddedComparableBytes = $embeddedBytes
    }

    $prefixText = '__TAURI_BUNDLE_TYPE_VAR_'
    $externalMarkerText = '__TAURI_BUNDLE_TYPE_VAR_UNK'
    $embeddedMarkerText = '__TAURI_BUNDLE_TYPE_VAR_NSS'
    $externalMarker = [System.Text.Encoding]::ASCII.GetBytes($externalMarkerText)
    $embeddedMarker = [System.Text.Encoding]::ASCII.GetBytes($embeddedMarkerText)
    $externalUnkOffsets = @(Find-ByteSequenceOffsets -Data $externalBytes -Pattern $externalMarker)
    $externalNssOffsets = @(Find-ByteSequenceOffsets -Data $externalBytes -Pattern $embeddedMarker)
    $embeddedUnkOffsets = @(Find-ByteSequenceOffsets -Data $embeddedComparableBytes -Pattern $externalMarker)
    $embeddedNssOffsets = @(Find-ByteSequenceOffsets -Data $embeddedComparableBytes -Pattern $embeddedMarker)
    if ($externalUnkOffsets.Count -ne 1 -or $externalNssOffsets.Count -ne 0) {
        throw 'External main executable must contain exactly one complete Tauri UNK bundle marker and no NSS marker.'
    }
    if ($embeddedNssOffsets.Count -ne 1 -or $embeddedUnkOffsets.Count -ne 0) {
        throw 'NSIS main payload must contain exactly one complete Tauri NSS bundle marker and no UNK marker.'
    }
    if ($externalUnkOffsets[0] -ne $embeddedNssOffsets[0]) {
        throw 'External and NSIS main executable Tauri bundle markers are at different offsets.'
    }

    $externalComparableBytes = [byte[]] $externalBytes.Clone()
    $suffixOffset = $embeddedNssOffsets[0] + [System.Text.Encoding]::ASCII.GetByteCount($prefixText)
    $replacement = [System.Text.Encoding]::ASCII.GetBytes('UNK')
    [System.Array]::Copy($replacement, 0, $embeddedComparableBytes, $suffixOffset, $replacement.Length)
    if ($AuthenticodeAware) {
        Clear-CheckedByteRange -Bytes $externalComparableBytes -Offset ([long] $externalLayout.checksumOffset) -Length 4 -Description 'external PE checksum'
        Clear-CheckedByteRange -Bytes $embeddedComparableBytes -Offset ([long] $embeddedLayout.checksumOffset) -Length 4 -Description 'embedded PE checksum'
        Clear-CheckedByteRange -Bytes $externalComparableBytes -Offset ([long] $externalLayout.securityDirectoryOffset) -Length 8 -Description 'external PE security directory'
        Clear-CheckedByteRange -Bytes $embeddedComparableBytes -Offset ([long] $embeddedLayout.securityDirectoryOffset) -Length 8 -Description 'embedded PE security directory'
    }
    $externalComparableHash = Get-ByteArraySha256 -Bytes $externalComparableBytes
    $normalizedHash = Get-ByteArraySha256 -Bytes $embeddedComparableBytes
    $externalHash = Get-Sha256 -LiteralPath $ExternalPath
    $rawEmbeddedHash = Get-Sha256 -LiteralPath $EmbeddedPath
    if (-not [string]::Equals($normalizedHash, $externalComparableHash, [System.StringComparison]::Ordinal)) {
        throw 'NSIS main payload differs outside the explicitly allowed Tauri marker and Authenticode fields.'
    }

    return [ordered]@{
        comparisonMode = if ($AuthenticodeAware) { 'authenticode-aware' } else { 'strict-unsigned' }
        rawEmbeddedSha256 = $rawEmbeddedHash
        normalizedEmbeddedSha256 = $normalizedHash
        comparableExternalSha256 = $externalComparableHash
        externalSha256 = $externalHash
        externalSizeBytes = $externalItem.Length
        sizeBytes = $embeddedItem.Length
        markerOffset = [long] $embeddedNssOffsets[0]
        embeddedMarker = $embeddedMarkerText
        normalizedMarker = $externalMarkerText
        normalizedMatchesExternal = $true
        certificateTable = $certificateReport
        excludedAuthenticodeFields = if ($AuthenticodeAware) {
            @('pe-checksum', 'security-directory-entry', 'align8-zero-padding', 'eof-win-certificate-table')
        }
        else {
            @()
        }
    }
}

function Get-NsisScriptPayloadReport {
    param(
        [Parameter(Mandatory = $true)] [string] $ScriptPath,
        [Parameter(Mandatory = $true)] [string] $MainExecutable,
        [Parameter(Mandatory = $true)] [object[]] $ExpectedPayload
    )

    $scriptText = Get-Content -Raw -LiteralPath $ScriptPath -Encoding UTF8
    if ($scriptText.IndexOf([char] 0) -ge 0) {
        throw 'Generated installer.nsi contains a NUL character.'
    }
    $lines = @($scriptText -split '\r?\n')

    $mainSourceMatches = @($lines | Where-Object { $_ -match '^\s*!define\s+MAINBINARYSRCPATH\s+"([^"]+)"\s*$' })
    $mainNameMatches = @($lines | Where-Object { $_ -match '^\s*!define\s+MAINBINARYNAME\s+"([^"]+)"\s*$' })
    if ($mainSourceMatches.Count -ne 1 -or $mainNameMatches.Count -ne 1) {
        throw 'Generated installer.nsi must define MAINBINARYSRCPATH and MAINBINARYNAME exactly once.'
    }
    [void] ($mainSourceMatches[0] -match '^\s*!define\s+MAINBINARYSRCPATH\s+"([^"]+)"\s*$')
    $declaredMainSource = Get-CanonicalExistingPath -LiteralPath $Matches[1] -RequireDirectory $false -Description 'installer.nsi main source'
    [void] ($mainNameMatches[0] -match '^\s*!define\s+MAINBINARYNAME\s+"([^"]+)"\s*$')
    $mainBinaryName = $Matches[1]
    if ($mainBinaryName -cne [System.IO.Path]::GetFileNameWithoutExtension($MainExecutable)) {
        throw "installer.nsi MAINBINARYNAME '$mainBinaryName' does not match the supplied main executable."
    }
    if (-not [string]::Equals($declaredMainSource, $MainExecutable, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'installer.nsi MAINBINARYSRCPATH does not resolve to the supplied main executable.'
    }

    $sectionStarts = [System.Collections.Generic.List[int]]::new()
    for ($index = 0; $index -lt $lines.Count; $index++) {
        if ($lines[$index] -match '^\s*Section\s+Install\s*$') {
            $sectionStarts.Add($index)
        }
    }
    if ($sectionStarts.Count -ne 1) {
        throw 'Generated installer.nsi must contain exactly one unquoted Section Install.'
    }
    $installStart = $sectionStarts[0]
    $installEnd = -1
    for ($index = $installStart + 1; $index -lt $lines.Count; $index++) {
        if ($lines[$index] -match '^\s*Section\b') {
            throw 'Generated installer.nsi starts another section before Section Install ends.'
        }
        if ($lines[$index] -match '^\s*SectionEnd\s*$') {
            $installEnd = $index
            break
        }
    }
    if ($installEnd -lt 0) {
        throw 'Generated installer.nsi Section Install is not terminated.'
    }

    $actualPayload = [System.Collections.Generic.Dictionary[string, object]]::new([System.StringComparer]::OrdinalIgnoreCase)
    $mainFileCount = 0
    for ($index = $installStart + 1; $index -lt $installEnd; $index++) {
        $line = $lines[$index]
        if ($line -notmatch '^\s*File\b') {
            continue
        }
        if ($line -match '^\s*File\s+"\$\{MAINBINARYSRCPATH\}"\s*$') {
            $mainFileCount++
            $destination = "$mainBinaryName.exe"
            if ($actualPayload.ContainsKey($destination)) {
                throw "installer.nsi declares duplicate payload destination '$destination'."
            }
            $actualPayload[$destination] = [ordered]@{
                destination = $destination
                sourcePath = $declaredMainSource
                lineNumber = $index + 1
            }
            continue
        }
        if ($line -notmatch '^\s*File\s+/a\s+"/oname=([^"]+)"\s+"([^"]+)"\s*$') {
            throw "installer.nsi has an unsupported File command in Section Install at line $($index + 1): $line"
        }
        $destination = ConvertTo-NsisPayloadPath -Value $Matches[1] -Description 'installer.nsi /oname destination'
        $sourcePath = Get-CanonicalExistingPath -LiteralPath $Matches[2] -RequireDirectory $false -Description "installer.nsi payload source '$destination'"
        if ($actualPayload.ContainsKey($destination)) {
            throw "installer.nsi declares duplicate payload destination '$destination'."
        }
        $actualPayload[$destination] = [ordered]@{
            destination = $destination
            sourcePath = $sourcePath
            lineNumber = $index + 1
        }
    }
    if ($mainFileCount -ne 1) {
        throw 'installer.nsi must include File "${MAINBINARYSRCPATH}" exactly once in Section Install.'
    }

    for ($index = 0; $index -lt $lines.Count; $index++) {
        if ($index -gt $installStart -and $index -lt $installEnd) { continue }
        if ($lines[$index] -match '^\s*File\b' -and
            $lines[$index] -notmatch '^\s*File\s+"/oname=\$TEMP\\MicrosoftEdgeWeb[vV]iew2(?:Setup|RuntimeInstaller)\.exe"\s+"\$\{WEBVIEW2(?:BOOTSTRAPPER|INSTALLER)PATH\}"\s*$') {
            throw "installer.nsi contains an unexpected File command outside Section Install at line $($index + 1)."
        }
    }

    if ($actualPayload.Count -ne $ExpectedPayload.Count) {
        throw "installer.nsi Install payload count mismatch. Expected $($ExpectedPayload.Count), found $($actualPayload.Count)."
    }
    $reports = [System.Collections.Generic.List[object]]::new()
    foreach ($expected in $ExpectedPayload) {
        $destination = ConvertTo-NsisPayloadPath -Value ([string] $expected.destination) -Description 'expected NSIS destination'
        if (-not $actualPayload.ContainsKey($destination)) {
            throw "installer.nsi is missing expected payload destination '$destination'."
        }
        $actual = $actualPayload[$destination]
        $actualItem = Get-Item -LiteralPath $actual.sourcePath -Force
        $actualHash = Get-Sha256 -LiteralPath $actual.sourcePath
        if ($actualItem.Length -ne [long] $expected.sizeBytes -or
            -not [string]::Equals($actualHash, [string] $expected.sha256, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "installer.nsi source for '$destination' does not match the expected payload bytes."
        }
        $reports.Add([ordered]@{
                destination = $destination
                sourcePath = $actual.sourcePath
                sizeBytes = $actualItem.Length
                sha256 = $actualHash
                lineNumber = $actual.lineNumber
            })
    }
    return [ordered]@{
        path = $ScriptPath
        sha256 = Get-Sha256 -LiteralPath $ScriptPath
        installSectionStartLine = $installStart + 1
        installSectionEndLine = $installEnd + 1
        entries = $reports.ToArray()
    }
}

function Assert-NsisArchivePayload {
    param(
        [Parameter(Mandatory = $true)] [string] $ExtractorPath,
        [Parameter(Mandatory = $true)] [string] $InstallerPath,
        [Parameter(Mandatory = $true)] [object] $NsisHeader,
        [Parameter(Mandatory = $true)] [object[]] $ExpectedPayload,
        [Parameter(Mandatory = $true)] [bool] $PermitUnsignedApplicationArtifacts,
        [AllowNull()] [object] $SigningRequirements,
        [AllowNull()] [object] $VerifiedOutputConfiguration
    )

    Assert-PeFile -LiteralPath $ExtractorPath -Description 'NSIS extractor'
    if ([System.IO.Path]::GetFileName($ExtractorPath) -cne '7z.exe') {
        throw "NSIS extractor must be the full 7-Zip 7z.exe, not 7za/7zr or an arbitrary executable: '$ExtractorPath'."
    }
    $extractorHash = Get-Sha256 -LiteralPath $ExtractorPath
    $listResult = Invoke-CheckedExternalProcess `
        -Executable $ExtractorPath `
        -Arguments @('l', '-slt', '-sccUTF-8', '--', $InstallerPath) `
        -TimeoutSeconds 300 `
        -Description '7-Zip NSIS listing'
    $divider = $listResult.stdout.IndexOf("----------", [System.StringComparison]::Ordinal)
    if ($divider -lt 0) {
        throw '7-Zip NSIS listing did not contain an entry divider.'
    }
    $archiveSummary = $listResult.stdout.Substring(0, $divider)
    if ($archiveSummary -notmatch '(?m)^Type = Nsis\r?$' -or
        $archiveSummary -notmatch '(?m)^SubType = NSIS-3 Unicode\r?$') {
        throw '7-Zip did not identify the installer as an NSIS 3 Unicode archive.'
    }
    if ($archiveSummary -notmatch '(?m)^Physical Size = (\d+)\r?$' -or [long] $Matches[1] -ne (Get-Item -LiteralPath $InstallerPath).Length) {
        throw '7-Zip NSIS physical size does not match the installer file.'
    }
    if ($archiveSummary -notmatch '(?m)^Headers Size = (\d+)\r?$' -or [long] $Matches[1] -ne [long] $NsisHeader.headerLength) {
        throw '7-Zip NSIS header size does not match the parsed first header.'
    }
    if ($archiveSummary -notmatch '(?m)^Embedded Stub Size = (\d+)\r?$' -or [long] $Matches[1] -ne [long] $NsisHeader.overlayOffset) {
        throw '7-Zip NSIS embedded-stub size does not match the PE overlay boundary.'
    }

    $entryText = $listResult.stdout.Substring($divider)
    $listedPaths = @([regex]::Matches($entryText, '(?m)^Path = ([^\r\n]+)\r?$') | ForEach-Object { $_.Groups[1].Value })
    foreach ($expected in $ExpectedPayload) {
        $destination = [string] $expected.destination
        $matches = @($listedPaths | Where-Object { [string]::Equals($_, $destination, [System.StringComparison]::OrdinalIgnoreCase) })
        if ($matches.Count -ne 1) {
            throw "NSIS archive must contain payload '$destination' exactly once; found $($matches.Count)."
        }
    }

    $tempRoot = if ($null -eq $VerifiedOutputConfiguration) {
        [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd('\')
    }
    else {
        Assert-VerifiedPayloadOutputBoundary -Configuration $VerifiedOutputConfiguration -RequireEmpty
        [System.IO.Directory]::GetParent([string] $VerifiedOutputConfiguration.path).FullName.TrimEnd('\')
    }
    $extractRoot = Join-Path $tempRoot ('vhm-nsis-payload-' + [Guid]::NewGuid().ToString('N'))
    [void] [System.IO.Directory]::CreateDirectory($extractRoot)
    $verifiedOutputRetained = $false
    $verifiedOutputVerification = $null
    $embeddedMainSignature = $null
    try {
        $arguments = [System.Collections.Generic.List[string]]::new()
        foreach ($argument in @('x', '-y', '-bd', '-bb0', '-aoa', '-sccUTF-8', "-o$extractRoot", '--', $InstallerPath)) {
            $arguments.Add($argument)
        }
        foreach ($expected in $ExpectedPayload) {
            $arguments.Add([string] $expected.destination)
        }
        [void] (Invoke-CheckedExternalProcess -Executable $ExtractorPath -Arguments $arguments.ToArray() -TimeoutSeconds 300 -Description '7-Zip controlled NSIS extraction')

        $allItems = @(Get-ChildItem -LiteralPath $extractRoot -Recurse -Force)
        foreach ($item in $allItems) {
            if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "Controlled NSIS extraction produced a reparse point: '$($item.FullName)'."
            }
        }
        $files = @($allItems | Where-Object { -not $_.PSIsContainer })
        if ($files.Count -ne $ExpectedPayload.Count) {
            throw "Controlled NSIS extraction produced $($files.Count) files; expected exactly $($ExpectedPayload.Count)."
        }

        $reports = [System.Collections.Generic.List[object]]::new()
        foreach ($expected in $ExpectedPayload) {
            $relativeNative = ([string] $expected.destination).Replace('\', [System.IO.Path]::DirectorySeparatorChar)
            $candidate = [System.IO.Path]::GetFullPath((Join-Path $extractRoot $relativeNative))
            if (-not (Test-PathWithinRoot -Root $extractRoot -Candidate $candidate)) {
                throw "Expected extracted payload escapes the controlled root: '$($expected.destination)'."
            }
            $item = Get-Item -LiteralPath $candidate -Force -ErrorAction Stop
            if ($item.PSIsContainer -or ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "Extracted NSIS payload is not a regular file: '$($expected.destination)'."
            }
            $hash = Get-Sha256 -LiteralPath $candidate
            $sourceItem = Get-Item -LiteralPath $expected.sourcePath -Force
            $sourceHash = Get-Sha256 -LiteralPath $expected.sourcePath
            if ($sourceItem.Length -ne [long] $expected.sizeBytes -or
                -not [string]::Equals($sourceHash, [string] $expected.sha256, [System.StringComparison]::OrdinalIgnoreCase)) {
                throw "Expected source bytes for '$($expected.destination)' changed during NSIS validation."
            }

            $comparison = 'exact'
            if ($expected -is [System.Collections.IDictionary] -and $expected.Contains('comparison')) {
                $comparison = [string] $expected['comparison']
            }
            elseif ($expected -isnot [System.Collections.IDictionary]) {
                $comparisonProperty = $expected.PSObject.Properties['comparison']
                if ($null -ne $comparisonProperty) {
                    $comparison = [string] $comparisonProperty.Value
                }
            }
            $entryReport = [ordered]@{
                destination = [string] $expected.destination
                sizeBytes = $item.Length
                sha256 = $hash
                comparison = $comparison
            }
            if ($comparison -ceq 'tauri-nsis-marker-normalized') {
                $normalization = Get-TauriNsisMainPayloadReport `
                    -ExternalPath ([string] $expected.sourcePath) `
                    -EmbeddedPath $candidate `
                    -AuthenticodeAware (-not $PermitUnsignedApplicationArtifacts)
                $entryReport['normalization'] = $normalization
                $embeddedMainSignature = Get-SignatureReport `
                    -LiteralPath $candidate `
                    -Description 'NSIS embedded main executable' `
                    -PermitUnsigned $PermitUnsignedApplicationArtifacts `
                    -SigningRequirements $SigningRequirements
            }
            elseif ($comparison -ceq 'exact') {
                if ($item.Length -ne [long] $expected.sizeBytes -or
                    -not [string]::Equals($hash, [string] $expected.sha256, [System.StringComparison]::OrdinalIgnoreCase)) {
                    throw "NSIS compressed payload '$($expected.destination)' does not match the expected source bytes."
                }
            }
            else {
                throw "Unsupported NSIS payload comparison mode '$comparison' for '$($expected.destination)'."
            }
            $reports.Add($entryReport)
        }
        if ($null -eq $embeddedMainSignature) {
            throw 'Controlled NSIS extraction did not validate an embedded main executable signature.'
        }

        $entryReports = $reports.ToArray()
        if ($null -ne $VerifiedOutputConfiguration) {
            Assert-VerifiedPayloadOutputBoundary -Configuration $VerifiedOutputConfiguration -RequireEmpty
            $outputPath = [System.IO.Path]::GetFullPath([string] $VerifiedOutputConfiguration.path)
            Remove-Item -LiteralPath $outputPath -Force
            [System.IO.Directory]::Move($extractRoot, $outputPath)
            $verifiedOutputRetained = $true
            Assert-VerifiedPayloadOutputBoundary -Configuration $VerifiedOutputConfiguration
            $verifiedOutputVerification = Assert-VerifiedPayloadMatchesReport `
                -Configuration $VerifiedOutputConfiguration `
                -EntryReports $entryReports
        }
        return [ordered]@{
            extractorPath = $ExtractorPath
            extractorSha256 = $extractorHash
            archiveType = 'Nsis'
            archiveSubType = 'NSIS-3 Unicode'
            verifiedPayloadOutputDirectory = if ($verifiedOutputRetained) { [string] $VerifiedOutputConfiguration.path } else { $null }
            verifiedPayloadOutputVerification = $verifiedOutputVerification
            embeddedMainSignature = $embeddedMainSignature
            entries = $entryReports
        }
    }
    finally {
        $extractFull = [System.IO.Path]::GetFullPath($extractRoot)
        $safeName = [System.IO.Path]::GetFileName($extractFull) -cmatch '^vhm-nsis-payload-[0-9a-f]{32}$'
        $safeParent = [string]::Equals([System.IO.Directory]::GetParent($extractFull).FullName.TrimEnd('\'), $tempRoot, [System.StringComparison]::OrdinalIgnoreCase)
        if (-not $verifiedOutputRetained -and $safeName -and $safeParent -and [System.IO.Directory]::Exists($extractFull)) {
            Remove-Item -LiteralPath $extractFull -Recurse -Force
        }
    }
}

function Get-CompliancePayloadReports {
    param(
        [Parameter(Mandatory = $true)] [string] $ResourceRoot,
        [Parameter(Mandatory = $true)] [string] $RelativeRoot,
        [Parameter(Mandatory = $true)] [string] $RepositoryRoot,
        [Parameter(Mandatory = $true)] [string] $FfmpegManifestPath,
        [Parameter(Mandatory = $true)] [ValidateSet('public', 'community-beta', 'personal-community-stable')] [string] $ExpectedReleaseProfile,
        [Parameter(Mandatory = $true)] [bool] $RequirePublicReady
    )

    $ffmpegManifestRelativePath = [System.IO.Path]::GetRelativePath(
        $RepositoryRoot,
        $FfmpegManifestPath
    ).Replace('\', '/')
    if ([System.IO.Path]::IsPathRooted($ffmpegManifestRelativePath) -or
        $ffmpegManifestRelativePath -eq '..' -or
        $ffmpegManifestRelativePath.StartsWith('../', [System.StringComparison]::Ordinal) -or
        $ffmpegManifestRelativePath.Contains(':') -or
        $ffmpegManifestRelativePath.IndexOf([char] 0) -ge 0) {
        throw 'FFmpeg compliance manifest input must remain inside the repository.'
    }

    $complianceRoot = Get-ResourceDirectory `
        -Root $ResourceRoot `
        -RelativePath $RelativeRoot `
        -Description 'third-party compliance directory'
    $manifestPath = Get-ResourceFile `
        -Root $complianceRoot `
        -RelativePath 'COMPLIANCE-MANIFEST.json' `
        -Description 'third-party compliance manifest'
    $manifest = Get-Content -Raw -LiteralPath $manifestPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
    if ([long] (Get-RequiredJsonProperty -Object $manifest -Name 'schemaVersion' -Context 'compliance manifest') -ne 1) {
        throw 'Unsupported compliance manifest schemaVersion.'
    }
    $manifestProfileProperty = $manifest.PSObject.Properties['releaseProfile']
    $manifestProfile = if ($null -eq $manifestProfileProperty) { 'public' } else { [string] $manifestProfileProperty.Value }
    if ($manifestProfile -cne $ExpectedReleaseProfile) {
        throw "Compliance manifest releaseProfile '$manifestProfile' does not match '$ExpectedReleaseProfile'."
    }
    if ([string] (Get-RequiredJsonProperty -Object $manifest -Name 'target' -Context 'compliance manifest') -cne 'x86_64-pc-windows-msvc') {
        throw 'Compliance manifest target must be x86_64-pc-windows-msvc.'
    }

    $declaredFiles = @(Get-RequiredJsonProperty -Object $manifest -Name 'files' -Context 'compliance manifest')
    $declaredFileCount = [long] (Get-RequiredJsonProperty -Object $manifest -Name 'fileCount' -Context 'compliance manifest')
    if ($declaredFiles.Count -le 0 -or $declaredFileCount -ne $declaredFiles.Count) {
        throw 'Compliance manifest fileCount must match a non-empty files array.'
    }

    $requiredFiles = @(
        'npm-runtime.spdx.json',
        'npm-build.spdx.json',
        'cargo-windows-x64.spdx.json',
        'ffmpeg-component.json',
        'LICENSE-TEXTS-INDEX.json',
        'THIRD-PARTY-LICENSES.txt',
        'THIRD-PARTY-NOTICES.md',
        'COMPLIANCE-SUMMARY.json'
    )
    $declaredPaths = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    $reports = [System.Collections.Generic.List[object]]::new()
    foreach ($declaration in $declaredFiles) {
        $relativePath = Get-RequiredJsonString -Object $declaration -Name 'path' -Context 'compliance manifest file'
        if ([System.IO.Path]::IsPathRooted($relativePath) -or $relativePath.Contains('\') -or
            $relativePath.Contains(':') -or $relativePath.IndexOf([char] 0) -ge 0) {
            throw "Compliance manifest contains an unsafe file path: '$relativePath'."
        }
        $segments = $relativePath.Split('/')
        if (@($segments | Where-Object { [string]::IsNullOrWhiteSpace($_) -or $_ -in @('.', '..') }).Count -ne 0 -or
            -not $declaredPaths.Add($relativePath)) {
            throw "Compliance manifest contains an unsafe or duplicate file path: '$relativePath'."
        }

        $filePath = Get-ResourceFile `
            -Root $complianceRoot `
            -RelativePath ($segments -join [System.IO.Path]::DirectorySeparatorChar) `
            -Description "compliance file '$relativePath'"
        $item = Get-Item -LiteralPath $filePath -Force
        $expectedSize = [long] (Get-RequiredJsonProperty -Object $declaration -Name 'sizeBytes' -Context "compliance file '$relativePath'")
        $expectedHash = Get-RequiredJsonString -Object $declaration -Name 'sha256' -Context "compliance file '$relativePath'"
        Assert-Sha256Text -Value $expectedHash -Description "compliance file '$relativePath' hash"
        $actualHash = Get-Sha256 -LiteralPath $filePath
        if ($expectedSize -ne $item.Length -or
            -not [string]::Equals($expectedHash, $actualHash, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Compliance file '$relativePath' does not match COMPLIANCE-MANIFEST.json."
        }
        $reports.Add([ordered]@{
                destination = (Join-Path $RelativeRoot ($segments -join '\'))
                sourcePath = $filePath
                sizeBytes = $item.Length
                sha256 = $actualHash
            })
    }
    foreach ($requiredFile in $requiredFiles) {
        if (-not $declaredPaths.Contains($requiredFile)) {
            throw "Compliance manifest is missing required file '$requiredFile'."
        }
    }

    $manifestItem = Get-Item -LiteralPath $manifestPath -Force
    $reports.Add([ordered]@{
            destination = (Join-Path $RelativeRoot 'COMPLIANCE-MANIFEST.json')
            sourcePath = $manifestPath
            sizeBytes = $manifestItem.Length
            sha256 = Get-Sha256 -LiteralPath $manifestPath
        })

    $actualItems = @(Get-ChildItem -LiteralPath $complianceRoot -Recurse -Force)
    foreach ($item in $actualItems) {
        if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Compliance directory contains a reparse point: '$($item.FullName)'."
        }
    }
    $actualFiles = @($actualItems | Where-Object { -not $_.PSIsContainer })
    if ($actualFiles.Count -ne ($declaredFiles.Count + 1)) {
        throw 'Compliance directory contains files not covered by COMPLIANCE-MANIFEST.json.'
    }

    $generator = Get-RequiredJsonProperty -Object $manifest -Name 'generator' -Context 'compliance manifest'
    $generatorRelativePath = Get-RequiredJsonString -Object $generator -Name 'path' -Context 'compliance generator'
    if ($generatorRelativePath -cne 'scripts/release/generate-compliance-evidence.mjs') {
        throw 'Compliance manifest generator path is not the approved release generator.'
    }
    $generatorPath = Get-ResourceFile -Root $RepositoryRoot -RelativePath $generatorRelativePath -Description 'compliance generator source'
    $generatorHash = Get-RequiredJsonString -Object $generator -Name 'sha256' -Context 'compliance generator'
    Assert-Sha256Text -Value $generatorHash -Description 'compliance generator hash'
    if (-not [string]::Equals($generatorHash, (Get-Sha256 -LiteralPath $generatorPath), [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'Compliance evidence was generated by a different generator revision.'
    }

    $licenseOverrideManifestRelativePath = 'third_party/licenses/license-text-overrides.json'
    $licenseOverrideApprovalRelativePath = 'third_party/licenses/license-text-override-approvals.json'
    $licenseOverrideManifestPath = Get-ResourceFile `
        -Root $RepositoryRoot `
        -RelativePath ($licenseOverrideManifestRelativePath.Replace('/', [System.IO.Path]::DirectorySeparatorChar)) `
        -Description 'tracked license-text override manifest'
    $licenseOverrideManifest = Get-Content -Raw -LiteralPath $licenseOverrideManifestPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
    if ([long] (Get-RequiredJsonProperty -Object $licenseOverrideManifest -Name 'schemaVersion' -Context 'license-text override manifest') -ne 1) {
        throw 'Unsupported license-text override manifest schemaVersion.'
    }
    $licenseOverrideApprovalPath = Get-ResourceFile `
        -Root $RepositoryRoot `
        -RelativePath ($licenseOverrideApprovalRelativePath.Replace('/', [System.IO.Path]::DirectorySeparatorChar)) `
        -Description 'structured license-text override approval manifest'
    $licenseOverrideApproval = Get-Content -Raw -LiteralPath $licenseOverrideApprovalPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
    if ([long] (Get-RequiredJsonProperty -Object $licenseOverrideApproval -Name 'schemaVersion' -Context 'license-text override approval manifest') -ne 1) {
        throw 'Unsupported license-text override approval manifest schemaVersion.'
    }
    [void] (Get-RequiredJsonString -Object $licenseOverrideApproval -Name 'purpose' -Context 'license-text override approval manifest')
    $approvalRecords = @(Get-RequiredJsonProperty -Object $licenseOverrideApproval -Name 'approvals' -Context 'license-text override approval manifest')
    $overrideTexts = @(Get-RequiredJsonProperty -Object $licenseOverrideManifest -Name 'texts' -Context 'license-text override manifest')
    if ($overrideTexts.Count -le 0) {
        throw 'License-text override manifest must contain tracked text files.'
    }
    $overrideInputPaths = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    $overrideTextIds = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    $overrideTextById = [System.Collections.Generic.Dictionary[string, object]]::new([System.StringComparer]::Ordinal)
    foreach ($overrideText in $overrideTexts) {
        $textId = Get-RequiredJsonString -Object $overrideText -Name 'id' -Context 'license-text override file'
        if ($textId -cnotmatch '^[a-z0-9][a-z0-9.-]*$' -or -not $overrideTextIds.Add($textId)) {
            throw "License-text override manifest contains an unsafe or duplicate text id: '$textId'."
        }
        $relativePath = Get-RequiredJsonString -Object $overrideText -Name 'path' -Context 'license-text override file'
        if (-not $relativePath.StartsWith('third_party/licenses/texts/', [System.StringComparison]::Ordinal) -or
            [System.IO.Path]::IsPathRooted($relativePath) -or $relativePath.Contains('\') -or
            $relativePath.Contains(':') -or $relativePath.IndexOf([char] 0) -ge 0) {
            throw "License-text override manifest contains an unsafe text path: '$relativePath'."
        }
        $segments = $relativePath.Split('/')
        if (@($segments | Where-Object { [string]::IsNullOrWhiteSpace($_) -or $_ -in @('.', '..') }).Count -ne 0 -or
            -not $overrideInputPaths.Add($relativePath)) {
            throw "License-text override manifest contains an unsafe or duplicate text path: '$relativePath'."
        }
        $textPath = Get-ResourceFile `
            -Root $RepositoryRoot `
            -RelativePath ($segments -join [System.IO.Path]::DirectorySeparatorChar) `
            -Description "tracked license-text override '$relativePath'"
        $textItem = Get-Item -LiteralPath $textPath -Force
        $expectedSize = [long] (Get-RequiredJsonProperty -Object $overrideText -Name 'sizeBytes' -Context "license-text override '$relativePath'")
        $expectedHash = Get-RequiredJsonString -Object $overrideText -Name 'sha256' -Context "license-text override '$relativePath'"
        Assert-Sha256Text -Value $expectedHash -Description "license-text override '$relativePath' hash"
        if ($expectedSize -ne $textItem.Length -or
            -not [string]::Equals($expectedHash, (Get-Sha256 -LiteralPath $textPath), [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "License-text override '$relativePath' does not match its manifest size and SHA-256."
        }
        $spdxProperty = $overrideText.PSObject.Properties['spdxLicenseId']
        if ($null -eq $spdxProperty -or
            ($null -ne $spdxProperty.Value -and
             ($spdxProperty.Value -isnot [string] -or [string] $spdxProperty.Value -cnotmatch '^[A-Za-z0-9][A-Za-z0-9.+-]*$' -or
              [string] $spdxProperty.Value -cin @('AND', 'OR', 'WITH')))) {
            throw "License-text override '$textId' has an invalid SPDX license id."
        }
        $overrideTextById.Add($textId, [ordered]@{
                sha256 = $expectedHash
                spdxLicenseId = if ($null -eq $spdxProperty.Value) { $null } else { [string] $spdxProperty.Value }
            })
    }

    $approvalByComponent = [System.Collections.Generic.Dictionary[string, object]]::new([System.StringComparer]::Ordinal)
    foreach ($approval in $approvalRecords) {
        $component = Get-RequiredJsonString -Object $approval -Name 'component' -Context 'license-text override approval'
        $decision = Get-RequiredJsonString -Object $approval -Name 'decision' -Context "license-text override approval '$component'"
        $reviewer = Get-RequiredJsonString -Object $approval -Name 'reviewer' -Context "license-text override approval '$component'"
        $reviewedAtUtc = Get-RequiredJsonString -Object $approval -Name 'reviewedAtUtc' -Context "license-text override approval '$component'"
        [void] (Get-RequiredJsonString -Object $approval -Name 'approvalReference' -Context "license-text override approval '$component'")
        $declaredLicense = Get-RequiredJsonString -Object $approval -Name 'declaredLicense' -Context "license-text override approval '$component'"
        $textHashes = @(Get-RequiredJsonProperty -Object $approval -Name 'textSha256' -Context "license-text override approval '$component'")
        $parsedReviewedAt = [DateTimeOffset]::MinValue
        $reviewedAtValid = [DateTimeOffset]::TryParse(
            $reviewedAtUtc,
            [System.Globalization.CultureInfo]::InvariantCulture,
            [System.Globalization.DateTimeStyles]::AssumeUniversal -bor [System.Globalization.DateTimeStyles]::AdjustToUniversal,
            [ref] $parsedReviewedAt
        )
        $uniqueHashes = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
        foreach ($textHash in $textHashes) {
            if ($textHash -isnot [string]) {
                throw "License-text override approval '$component' contains a non-string text hash."
            }
            Assert-Sha256Text -Value ([string] $textHash) -Description "license-text override approval '$component' text hash"
            if (-not $uniqueHashes.Add([string] $textHash)) {
                throw "License-text override approval '$component' contains duplicate text hashes."
            }
        }
        $sortedHashes = @($textHashes | Sort-Object -CaseSensitive)
        if ($component -cnotmatch '^(npm|cargo):[^@\x00]+@[^@\x00]+$' -or
            $decision -cne 'approved' -or
            $reviewedAtUtc -cnotmatch '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{3})?Z$' -or
            -not $reviewedAtValid -or
            $textHashes.Count -le 0 -or
            ($textHashes -join "`0") -cne ($sortedHashes -join "`0") -or
            -not $approvalByComponent.TryAdd($component, [ordered]@{
                    declaredLicense = $declaredLicense
                    textSha256 = $sortedHashes
                    reviewer = $reviewer
                    reviewedAtUtc = $reviewedAtUtc
                })) {
            throw "License-text override approval '$component' is invalid or duplicated."
        }
    }

    $overrideComponents = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    $usedOverrideTextIds = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    $pendingOverrideComponents = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    $overrideDeclarations = @(Get-RequiredJsonProperty -Object $licenseOverrideManifest -Name 'overrides' -Context 'license-text override manifest')
    if ($overrideDeclarations.Count -le 0) {
        throw 'License-text override manifest must contain component mappings.'
    }
    foreach ($override in $overrideDeclarations) {
        if ($null -ne $override.PSObject.Properties['review']) {
            throw 'License-text override decisions must live only in the structured approval manifest.'
        }
        $ecosystem = Get-RequiredJsonString -Object $override -Name 'ecosystem' -Context 'license-text component override'
        $name = Get-RequiredJsonString -Object $override -Name 'name' -Context 'license-text component override'
        $version = Get-RequiredJsonString -Object $override -Name 'version' -Context 'license-text component override'
        $declaredLicense = Get-RequiredJsonString -Object $override -Name 'declaredLicense' -Context 'license-text component override'
        $component = '{0}:{1}@{2}' -f $ecosystem, $name, $version
        if ($ecosystem -cnotin @('npm', 'cargo') -or -not $overrideComponents.Add($component)) {
            throw "License-text override component '$component' is invalid or duplicated."
        }
        $textIds = @(Get-RequiredJsonProperty -Object $override -Name 'textIds' -Context "license-text override '$component'")
        $componentTextIds = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
        $componentTextHashes = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
        $coveredSpdxIds = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
        foreach ($textIdValue in $textIds) {
            if ($textIdValue -isnot [string] -or
                -not $componentTextIds.Add([string] $textIdValue) -or
                -not $overrideTextById.ContainsKey([string] $textIdValue)) {
                throw "License-text override '$component' references an unknown or duplicate text id."
            }
            [void] $usedOverrideTextIds.Add([string] $textIdValue)
            $textRecord = $overrideTextById[[string] $textIdValue]
            [void] $componentTextHashes.Add([string] $textRecord.sha256)
            if ($null -ne $textRecord.spdxLicenseId) {
                [void] $coveredSpdxIds.Add([string] $textRecord.spdxLicenseId)
            }
        }
        if ($textIds.Count -le 0 -or $declaredLicense -cnotmatch '^[A-Za-z0-9.+()\-\s]+$' -or $declaredLicense -cmatch '\bWITH\b') {
            throw "License-text override '$component' has an unsupported SPDX expression or no text ids."
        }
        $declaredSpdxIds = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
        foreach ($match in [regex]::Matches($declaredLicense, '[A-Za-z0-9][A-Za-z0-9.+-]*')) {
            if ($match.Value -cnotin @('AND', 'OR')) {
                [void] $declaredSpdxIds.Add($match.Value)
            }
        }
        if ($declaredSpdxIds.Count -le 0 -or
            @($declaredSpdxIds | Where-Object { -not $coveredSpdxIds.Contains($_) }).Count -ne 0 -or
            @($coveredSpdxIds | Where-Object { -not $declaredSpdxIds.Contains($_) }).Count -ne 0) {
            throw "License-text override '$component' does not have exact SPDX text coverage."
        }
        if ($approvalByComponent.ContainsKey($component)) {
            $approvalRecord = $approvalByComponent[$component]
            $actualHashes = @($componentTextHashes | Sort-Object -CaseSensitive)
            if ([string] $approvalRecord.declaredLicense -cne $declaredLicense -or
                (@($approvalRecord.textSha256) -join "`0") -cne ($actualHashes -join "`0")) {
                throw "License-text override approval '$component' does not match the current component texts."
            }
            [void] $approvalByComponent.Remove($component)
        }
        else {
            [void] $pendingOverrideComponents.Add($component)
        }
    }
    if ($approvalByComponent.Count -ne 0) {
        throw 'License-text override approval manifest contains unknown or stale components.'
    }
    if ($usedOverrideTextIds.Count -ne $overrideTextIds.Count -or
        @($overrideTextIds | Where-Object { -not $usedOverrideTextIds.Contains($_) }).Count -ne 0) {
        throw 'License-text override manifest contains text records that are not bound to a component.'
    }

    $requiredInputs = [System.Collections.Generic.List[string]]::new()
    foreach ($requiredInput in @(
            'package.json',
            'package-lock.json',
            'src-tauri/Cargo.toml',
            'src-tauri/Cargo.lock',
            $ffmpegManifestRelativePath,
            $licenseOverrideManifestRelativePath,
            $licenseOverrideApprovalRelativePath
        )) {
        $requiredInputs.Add($requiredInput)
    }
    foreach ($overrideInputPath in @($overrideInputPaths | Sort-Object)) {
        $requiredInputs.Add($overrideInputPath)
    }
    $inputDeclarations = @(Get-RequiredJsonProperty -Object $manifest -Name 'inputs' -Context 'compliance manifest')
    $inputPaths = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    foreach ($input in $inputDeclarations) {
        $inputRelativePath = Get-RequiredJsonString -Object $input -Name 'path' -Context 'compliance input'
        if (-not $inputPaths.Add($inputRelativePath)) {
            throw "Compliance manifest contains duplicate input '$inputRelativePath'."
        }
        $inputPath = Get-ResourceFile -Root $RepositoryRoot -RelativePath $inputRelativePath -Description "compliance input '$inputRelativePath'"
        $inputItem = Get-Item -LiteralPath $inputPath -Force
        $inputSize = [long] (Get-RequiredJsonProperty -Object $input -Name 'sizeBytes' -Context "compliance input '$inputRelativePath'")
        $inputHash = Get-RequiredJsonString -Object $input -Name 'sha256' -Context "compliance input '$inputRelativePath'"
        Assert-Sha256Text -Value $inputHash -Description "compliance input '$inputRelativePath' hash"
        if ($inputItem.Length -ne $inputSize -or
            -not [string]::Equals((Get-Sha256 -LiteralPath $inputPath), $inputHash, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Compliance evidence is stale for input '$inputRelativePath'."
        }
    }
    if ($inputPaths.Count -ne $requiredInputs.Count -or
        @($requiredInputs | Where-Object { -not $inputPaths.Contains($_) }).Count -ne 0) {
        throw 'Compliance manifest inputs do not exactly match the required release inputs.'
    }

    $summaryPath = Get-ResourceFile -Root $complianceRoot -RelativePath 'COMPLIANCE-SUMMARY.json' -Description 'compliance summary'
    $summary = Get-Content -Raw -LiteralPath $summaryPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
    if ([long] (Get-RequiredJsonProperty -Object $summary -Name 'schemaVersion' -Context 'compliance summary') -ne 1) {
        throw 'Unsupported compliance summary schemaVersion.'
    }
    $summaryProfileProperty = $summary.PSObject.Properties['releaseProfile']
    $summaryProfile = if ($null -eq $summaryProfileProperty) { 'public' } else { [string] $summaryProfileProperty.Value }
    if ($summaryProfile -cne $ExpectedReleaseProfile) {
        throw "Compliance summary releaseProfile '$summaryProfile' does not match '$ExpectedReleaseProfile'."
    }
    if ($ExpectedReleaseProfile -cin @('community-beta', 'personal-community-stable')) {
        $productionApproval = Get-RequiredJsonProperty -Object $summary -Name 'productionApproval' -Context 'compliance summary'
        if ($productionApproval -isnot [bool] -or [bool] $productionApproval) {
            throw "$ExpectedReleaseProfile compliance evidence must explicitly keep productionApproval false."
        }
    }
    $publicReady = Get-RequiredJsonProperty -Object $summary -Name 'publicRedistributionReady' -Context 'compliance summary'
    if ($publicReady -isnot [bool]) {
        throw 'Compliance summary publicRedistributionReady must be a Boolean.'
    }
    $blockers = @(Get-RequiredJsonProperty -Object $summary -Name 'blockers' -Context 'compliance summary')
    $overrideReviewItems = $blockers
    $channelDistributionReady = $false
    if ($ExpectedReleaseProfile -ceq 'personal-community-stable') {
        $channelReadyValue = Get-RequiredJsonProperty -Object $summary -Name 'channelDistributionReady' -Context 'compliance summary'
        if ($channelReadyValue -isnot [bool]) {
            throw 'Personal community stable compliance summary channelDistributionReady must be a Boolean.'
        }
        $channelDistributionReady = [bool] $channelReadyValue
        $overrideReviewItems = @(Get-RequiredJsonProperty -Object $summary -Name 'advisories' -Context 'compliance summary')
    }
    $pendingBlockerComponents = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    foreach ($blocker in $overrideReviewItems) {
        $code = Get-RequiredJsonString -Object $blocker -Name 'code' -Context 'compliance blocker'
        if ($code -cin @('NPM_LICENSE_OVERRIDE_REVIEW_PENDING', 'CARGO_LICENSE_OVERRIDE_REVIEW_PENDING')) {
            $blockedComponent = Get-RequiredJsonString -Object $blocker -Name 'component' -Context "compliance blocker '$code'"
            $ecosystemPrefix = if ($code -ceq 'NPM_LICENSE_OVERRIDE_REVIEW_PENDING') { 'npm:' } else { 'cargo:' }
            if (-not $pendingBlockerComponents.Add($ecosystemPrefix + $blockedComponent)) {
                throw 'Compliance summary contains duplicate license override pending blockers.'
            }
        }
    }
    if ($pendingBlockerComponents.Count -ne $pendingOverrideComponents.Count -or
        @($pendingOverrideComponents | Where-Object { -not $pendingBlockerComponents.Contains($_) }).Count -ne 0) {
        throw 'Compliance summary pending blockers do not match the structured license override approvals.'
    }
    if ($ExpectedReleaseProfile -ceq 'personal-community-stable') {
        if ([bool] $publicReady -or
            -not $channelDistributionReady -or
            $blockers.Count -ne 0 -or
            [string] $summary.status -cne 'ready-for-channel') {
            throw 'Personal community stable compliance evidence must be channel-ready with no technical blockers while keeping strict public redistribution unapproved.'
        }
    }
    else {
        $expectedReady = $blockers.Count -eq 0
        if ([bool] $publicReady -ne $expectedReady -or
            ([bool] $publicReady -and [string] $summary.status -cne 'ready-for-approval') -or
            (-not [bool] $publicReady -and [string] $summary.status -cne 'generated-with-blockers')) {
            throw 'Compliance summary readiness state is internally inconsistent.'
        }
    }

    foreach ($spdxFile in @('npm-runtime.spdx.json', 'npm-build.spdx.json', 'cargo-windows-x64.spdx.json')) {
        $spdxPath = Get-ResourceFile -Root $complianceRoot -RelativePath $spdxFile -Description "SPDX document '$spdxFile'"
        $spdx = Get-Content -Raw -LiteralPath $spdxPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
        if ([string] $spdx.spdxVersion -cne 'SPDX-2.3' -or
            [string] $spdx.dataLicense -cne 'CC0-1.0' -or
            @(Get-RequiredJsonProperty -Object $spdx -Name 'packages' -Context "SPDX document '$spdxFile'").Count -lt 2 -or
            @(Get-RequiredJsonProperty -Object $spdx -Name 'relationships' -Context "SPDX document '$spdxFile'").Count -eq 0) {
            throw "SPDX document '$spdxFile' is incomplete or uses an unsupported format."
        }
    }

    $ffmpegComponentPath = Get-ResourceFile -Root $complianceRoot -RelativePath 'ffmpeg-component.json' -Description 'FFmpeg component evidence'
    $ffmpegComponent = Get-Content -Raw -LiteralPath $ffmpegComponentPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
    $recordedFfmpegManifestHash = Get-RequiredJsonString -Object $ffmpegComponent -Name 'manifestSha256' -Context 'FFmpeg component evidence'
    Assert-Sha256Text -Value $recordedFfmpegManifestHash -Description 'FFmpeg component manifest hash'
    $currentFfmpegManifestPath = Get-ResourceFile -Root $RepositoryRoot -RelativePath $ffmpegManifestRelativePath -Description 'FFmpeg provenance manifest'
    if (-not [string]::Equals($recordedFfmpegManifestHash, (Get-Sha256 -LiteralPath $currentFfmpegManifestPath), [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'FFmpeg component evidence does not match the current FFmpeg provenance manifest.'
    }
    if ($RequirePublicReady -and (-not [bool] $publicReady -or [string] $summary.status -cne 'ready-for-approval')) {
        throw 'PUBLIC_COMPLIANCE_BLOCKED: third-party compliance summary is not ready for approval.'
    }

    return [ordered]@{
        root = $complianceRoot
        manifestPath = $manifestPath
        manifestSha256 = Get-Sha256 -LiteralPath $manifestPath
        publicRedistributionReady = [bool] $publicReady
        channelDistributionReady = $channelDistributionReady
        fileCount = $reports.Count
        entries = $reports.ToArray()
    }
}

if (-not (Test-IsWindows)) {
    throw 'check-bundle.ps1 must run on Windows because it validates Windows PE and Authenticode artifacts.'
}

$mainExecutable = Get-CanonicalExistingPath -LiteralPath $MainExecutablePath -RequireDirectory $false -Description 'main executable'
$nsisBundle = Get-CanonicalExistingPath -LiteralPath $NsisBundlePath -RequireDirectory $false -Description 'NSIS installer'
$nsisScript = Get-CanonicalExistingPath -LiteralPath $NsisScriptPath -RequireDirectory $false -Description 'generated installer.nsi'
$resourceRoot = Get-CanonicalExistingPath -LiteralPath $ResourceDirectory -RequireDirectory $true -Description 'resource directory'
$manifestPath = Get-CanonicalExistingPath -LiteralPath $FfmpegManifestPath -RequireDirectory $false -Description 'FFmpeg manifest'
$repositoryRoot = Get-CanonicalExistingPath -LiteralPath (Join-Path $PSScriptRoot '..\..') -RequireDirectory $true -Description 'repository root'
$projectLicense = Get-CanonicalExistingPath -LiteralPath (Join-Path $repositoryRoot 'LICENSE') -RequireDirectory $false -Description 'project MIT license'
$projectLicenseRelativePath = 'licenses\project\LICENSE.txt'
$projectLicenseScope = Get-CanonicalExistingPath -LiteralPath (Join-Path $repositoryRoot 'LICENSE-SCOPE.txt') -RequireDirectory $false -Description 'project license scope'
$projectLicenseScopeRelativePath = 'licenses\project\LICENSE-SCOPE.txt'
$releasePolicyPath = Get-CanonicalExistingPath -LiteralPath $PublicReleasePolicyPath -RequireDirectory $false -Description 'public release policy'
$approvedReleasePolicyPath = Get-CanonicalExistingPath -LiteralPath (Join-Path $repositoryRoot 'release\public-release-policy.json') -RequireDirectory $false -Description 'approved public release policy'
if (-not [string]::Equals($releasePolicyPath, $approvedReleasePolicyPath, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw 'PublicReleasePolicyPath must resolve to the repository approved release/public-release-policy.json.'
}
$communityBetaDecision = $null
$communityBetaFfmpegArchive = $null
$communityBetaSourceBundle = $null
$personalCommunityStableDecision = $null
# Note: no local assignment to $personalCommunityStableDecisionPath here.
# PowerShell variable names are case-insensitive, so nulling that name would
# erase the bound -PersonalCommunityStableDecisionPath parameter before the
# channel check below.
$personalCommunityStableFfmpegArchive = $null
$personalCommunityStableSourceBundle = $null
$personalCommunityStableVersion = $null
$personalCommunityStableTag = $null
if ($AllowUnsignedCommunityBeta) {
    if ([string]::IsNullOrWhiteSpace($CommunityBetaFfmpegArchivePath) -or
        [string]::IsNullOrWhiteSpace($CommunityBetaSourceBundlePath)) {
        throw 'Community Beta verification requires both FFmpeg binary and corresponding-source archive paths.'
    }
    $communityBetaDecisionPath = Get-CanonicalExistingPath -LiteralPath $CommunityBetaDecisionPath -RequireDirectory $false -Description 'Community Beta decision record'
    $approvedCommunityBetaDecisionPath = Get-CanonicalExistingPath -LiteralPath (Join-Path $repositoryRoot 'release\approvals\community-beta-v0.1.0.json') -RequireDirectory $false -Description 'approved Community Beta decision record'
    if (-not [string]::Equals($communityBetaDecisionPath, $approvedCommunityBetaDecisionPath, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'CommunityBetaDecisionPath must resolve to release/approvals/community-beta-v0.1.0.json.'
    }
    $communityBetaDecision = Get-Content -Raw -LiteralPath $communityBetaDecisionPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
    if ([long] (Get-RequiredJsonProperty -Object $communityBetaDecision -Name 'schemaVersion' -Context 'Community Beta decision') -ne 1 -or
        (Get-RequiredJsonString -Object $communityBetaDecision -Name 'channel' -Context 'Community Beta decision') -cne 'community-beta' -or
        (Get-RequiredJsonProperty -Object $communityBetaDecision -Name 'strictPublicReleaseApproval' -Context 'Community Beta decision') -isnot [bool] -or
        [bool] $communityBetaDecision.strictPublicReleaseApproval) {
        throw 'Community Beta decision must be schema v1, target community-beta, and explicitly not grant strict public approval.'
    }
    $betaConfirmations = Get-RequiredJsonProperty -Object $communityBetaDecision -Name 'releaseOwnerConfirmations' -Context 'Community Beta decision'
    foreach ($confirmation in @(
            'gameImagesMayBeDistributedInThisChannel',
            'gameContentScreenshotMayBePublishedInRepositoryReadme',
            'projectBrandIconMayBeDistributedInThisChannel',
            'unofficialProjectDisclaimerApprovedForThisChannel',
            'ffmpegMinimalBuildMayBeDistributedInThisChannel',
            'codecPatentReviewDeferredToStrictRelease',
            'automaticUpdatesAreDisabled',
            'installerIsUnsigned',
            'ffmpegUseIsLimitedToThumbnailGeneration'
        )) {
        $value = Get-RequiredJsonProperty -Object $betaConfirmations -Name $confirmation -Context 'Community Beta releaseOwnerConfirmations'
        if ($value -isnot [bool] -or -not [bool] $value) {
            throw "Community Beta decision confirmation '$confirmation' must be true."
        }
    }
    $betaRequirements = Get-RequiredJsonProperty -Object $communityBetaDecision -Name 'distributionRequirements' -Context 'Community Beta decision'
    foreach ($requirement in @(
            'windowsUnsignedWarningMustBeDisclosed',
            'manualUpdateInstructionsMustBeDisclosed',
            'ffmpegLicenseMaterialsMustAccompanyInstaller',
            'ffmpegBinaryAndBuildEvidenceMustAccompanyInstaller',
            'ffmpegCorrespondingSourceMustAccompanyInstaller',
            'communityBetaLimitationsMustBeDisclosed'
        )) {
        $value = Get-RequiredJsonProperty -Object $betaRequirements -Name $requirement -Context 'Community Beta distributionRequirements'
        if ($value -isnot [bool] -or -not [bool] $value) {
            throw "Community Beta distribution requirement '$requirement' must be true."
        }
    }
    $communityBetaFfmpegArchive = Get-CanonicalExistingPath -LiteralPath $CommunityBetaFfmpegArchivePath -RequireDirectory $false -Description 'Community Beta FFmpeg binary archive'
    $communityBetaSourceBundle = Get-CanonicalExistingPath -LiteralPath $CommunityBetaSourceBundlePath -RequireDirectory $false -Description 'Community Beta FFmpeg corresponding-source archive'
}
elseif ($AllowPersonalCommunityStable) {
    if ([string]::IsNullOrWhiteSpace($PersonalCommunityStableDecisionPath) -or
        [string]::IsNullOrWhiteSpace($FFmpegArchivePath) -or
        [string]::IsNullOrWhiteSpace($SourceBundlePath)) {
        throw 'Personal community stable verification requires the owner decision, FFmpeg binary archive, and corresponding-source archive paths.'
    }

    $packageJsonPath = Get-CanonicalExistingPath -LiteralPath (Join-Path $repositoryRoot 'package.json') -RequireDirectory $false -Description 'repository package.json'
    $packageJson = Get-Content -Raw -LiteralPath $packageJsonPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
    $personalCommunityStableVersion = Get-RequiredJsonString -Object $packageJson -Name 'version' -Context 'repository package.json'
    if ($personalCommunityStableVersion -cnotmatch '^\d+\.\d+\.\d+$') {
        throw "Repository package version is not a supported release version: '$personalCommunityStableVersion'."
    }
    $personalCommunityStableTag = "v$personalCommunityStableVersion"
    $personalCommunityStableDecisionPath = Get-CanonicalExistingPath -LiteralPath $PersonalCommunityStableDecisionPath -RequireDirectory $false -Description 'personal community stable decision record'
    $approvedPersonalCommunityStableDecisionPath = Get-CanonicalExistingPath `
        -LiteralPath (Join-Path $repositoryRoot "release\approvals\personal-community-stable-$personalCommunityStableTag.json") `
        -RequireDirectory $false `
        -Description 'approved personal community stable decision record'
    if (-not [string]::Equals($personalCommunityStableDecisionPath, $approvedPersonalCommunityStableDecisionPath, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "PersonalCommunityStableDecisionPath must resolve to release/approvals/personal-community-stable-$personalCommunityStableTag.json."
    }

    $personalCommunityStableDecision = Get-Content -Raw -LiteralPath $personalCommunityStableDecisionPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
    $strictPublicApproval = Get-RequiredJsonProperty -Object $personalCommunityStableDecision -Name 'strictPublicReleaseApproval' -Context 'personal community stable decision'
    $independentLegalReview = Get-RequiredJsonProperty -Object $personalCommunityStableDecision -Name 'independentLegalReviewCompleted' -Context 'personal community stable decision'
    if ([long] (Get-RequiredJsonProperty -Object $personalCommunityStableDecision -Name 'schemaVersion' -Context 'personal community stable decision') -ne 1 -or
        (Get-RequiredJsonString -Object $personalCommunityStableDecision -Name 'version' -Context 'personal community stable decision') -cne $personalCommunityStableVersion -or
        (Get-RequiredJsonString -Object $personalCommunityStableDecision -Name 'tag' -Context 'personal community stable decision') -cne $personalCommunityStableTag -or
        (Get-RequiredJsonString -Object $personalCommunityStableDecision -Name 'channel' -Context 'personal community stable decision') -cne 'personal-community-stable' -or
        (Get-RequiredJsonString -Object $personalCommunityStableDecision -Name 'decision' -Context 'personal community stable decision') -cne 'approved-by-repository-release-owner' -or
        (Get-RequiredJsonString -Object $personalCommunityStableDecision -Name 'decisionAuthority' -Context 'personal community stable decision') -cne 'repository-release-owner' -or
        (Get-RequiredJsonString -Object $personalCommunityStableDecision -Name 'distributionPurpose' -Context 'personal community stable decision') -cne 'free-personal-community' -or
        $strictPublicApproval -isnot [bool] -or [bool] $strictPublicApproval -or
        $independentLegalReview -isnot [bool] -or [bool] $independentLegalReview) {
        throw 'Personal community stable decision must bind the repository version/tag/channel to the release owner while honestly retaining strict-public and independent-legal-review status as false.'
    }

    $personalDistributionScope = Get-RequiredJsonProperty -Object $personalCommunityStableDecision -Name 'distributionScope' -Context 'personal community stable decision'
    foreach ($scopeConfirmation in @(
            'freeOfCharge',
            'nonCommercialCommunityProject',
            'githubStableRelease',
            'publicWindowsInstaller',
            'inAppStableUpdater'
        )) {
        $value = Get-RequiredJsonProperty -Object $personalDistributionScope -Name $scopeConfirmation -Context 'personal community stable distributionScope'
        if ($value -isnot [bool] -or -not [bool] $value) {
            throw "Personal community stable distribution scope '$scopeConfirmation' must be true."
        }
    }

    $personalConfirmations = Get-RequiredJsonProperty -Object $personalCommunityStableDecision -Name 'releaseOwnerConfirmations' -Context 'personal community stable decision'
    foreach ($confirmation in @(
            'manifestExactGameImagesMayBeDistributedInThisChannel',
            'projectBrandIconMayBeDistributedInThisChannel',
            'unofficialProjectDisclaimerRequired',
            'minimalLgplFfmpegMayBeDistributedInThisChannel',
            'ffmpegUseIsLimitedToThumbnailGeneration',
            'authenticodeMayBeDeferred',
            'tauriUpdaterSignatureRemainsRequired'
        )) {
        $value = Get-RequiredJsonProperty -Object $personalConfirmations -Name $confirmation -Context 'personal community stable releaseOwnerConfirmations'
        if ($value -isnot [bool] -or -not [bool] $value) {
            throw "Personal community stable decision confirmation '$confirmation' must be true."
        }
    }

    $personalRequirements = Get-RequiredJsonProperty -Object $personalCommunityStableDecision -Name 'distributionRequirements' -Context 'personal community stable decision'
    foreach ($requirement in @(
            'windowsUnsignedWarningMustBeDisclosed',
            'ffmpegLicenseMaterialsMustAccompanyInstaller',
            'ffmpegBinaryAndBuildEvidenceMustAccompanyRelease',
            'ffmpegCorrespondingSourceMustAccompanyRelease',
            'sha256ManifestMustCoverEveryOtherReleaseAsset',
            'thirdPartyNoticesMustAccompanyInstaller',
            'personalCommunityLimitationsMustBeDisclosed'
        )) {
        $value = Get-RequiredJsonProperty -Object $personalRequirements -Name $requirement -Context 'personal community stable distributionRequirements'
        if ($value -isnot [bool] -or -not [bool] $value) {
            throw "Personal community stable distribution requirement '$requirement' must be true."
        }
    }

    $personalAssetSet = Get-RequiredJsonProperty -Object $personalCommunityStableDecision -Name 'assetSet' -Context 'personal community stable decision'
    $personalAssetManifestRelativePath = Get-RequiredJsonString -Object $personalAssetSet -Name 'manifest' -Context 'personal community stable assetSet'
    if ($personalAssetManifestRelativePath -cne 'src/data/valorantAssets.json') {
        throw 'Personal community stable assetSet must bind src/data/valorantAssets.json.'
    }
    $personalAssetManifestPath = Get-CanonicalExistingPath `
        -LiteralPath (Join-Path $repositoryRoot $personalAssetManifestRelativePath) `
        -RequireDirectory $false `
        -Description 'personal community stable game asset manifest'
    if (-not (Test-PathWithinRoot -Root $repositoryRoot -Candidate $personalAssetManifestPath)) {
        throw 'Personal community stable game asset manifest escapes the repository root.'
    }
    Assert-NoReparsePoint -Root $repositoryRoot -Target $personalAssetManifestPath -Description 'personal community stable game asset manifest'
    $personalAssetManifestSha256 = Get-RequiredJsonString -Object $personalAssetSet -Name 'manifestSha256' -Context 'personal community stable assetSet'
    Assert-Sha256Text -Value $personalAssetManifestSha256 -Description 'personal community stable game asset manifest SHA-256'
    if ((Get-Sha256 -LiteralPath $personalAssetManifestPath) -cne $personalAssetManifestSha256) {
        throw 'Personal community stable game asset manifest SHA-256 does not match the owner decision.'
    }
    $personalAssetManifest = Get-Content -Raw -LiteralPath $personalAssetManifestPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
    $personalAssetCount = @($personalAssetManifest.agents).Count + @($personalAssetManifest.maps).Count
    if ([long] (Get-RequiredJsonProperty -Object $personalAssetSet -Name 'assetCount' -Context 'personal community stable assetSet') -ne $personalAssetCount) {
        throw 'Personal community stable game asset count does not match the owner decision.'
    }
    $personalCollectionFingerprint = Get-RequiredJsonString -Object $personalAssetSet -Name 'collectionFingerprint' -Context 'personal community stable assetSet'
    Assert-Sha256Text -Value $personalCollectionFingerprint -Description 'personal community stable game asset collection fingerprint'
    if ((Get-RequiredJsonString -Object $personalAssetManifest -Name 'collectionFingerprint' -Context 'game asset manifest') -cne $personalCollectionFingerprint) {
        throw 'Personal community stable game asset collection fingerprint does not match the owner decision.'
    }
    $sourceAssetBytesMustRemainManifestExact = Get-RequiredJsonProperty -Object $personalAssetSet -Name 'sourceAssetBytesMustRemainManifestExact' -Context 'personal community stable assetSet'
    if ($sourceAssetBytesMustRemainManifestExact -isnot [bool] -or -not [bool] $sourceAssetBytesMustRemainManifestExact) {
        throw 'Personal community stable owner decision must require manifest-exact game asset bytes.'
    }

    $personalCommunityStableFfmpegArchive = Get-CanonicalExistingPath -LiteralPath $FFmpegArchivePath -RequireDirectory $false -Description 'personal community stable FFmpeg binary archive'
    $personalCommunityStableSourceBundle = Get-CanonicalExistingPath -LiteralPath $SourceBundlePath -RequireDirectory $false -Description 'personal community stable FFmpeg corresponding-source archive'
}
$verifiedPayloadOutput = Resolve-VerifiedPayloadOutput -ConfiguredPath $VerifiedPayloadOutputDirectory

$resourceRootItem = Get-Item -LiteralPath $resourceRoot -Force
if (($resourceRootItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw "Resource directory must not be a reparse point: '$resourceRoot'."
}
foreach ($inputFile in @(
        [ordered]@{ path = $mainExecutable; description = 'main executable' },
        [ordered]@{ path = $nsisBundle; description = 'NSIS installer' },
        [ordered]@{ path = $nsisScript; description = 'generated installer.nsi' },
        [ordered]@{ path = $manifestPath; description = 'FFmpeg manifest' },
        [ordered]@{ path = $projectLicense; description = 'project MIT license' },
        [ordered]@{ path = $projectLicenseScope; description = 'project license scope' },
        [ordered]@{ path = $releasePolicyPath; description = 'public release policy' }
    )) {
    $inputItem = Get-Item -LiteralPath $inputFile.path -Force
    if (($inputItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$($inputFile.description) must not be a reparse point: '$($inputFile.path)'."
    }
}
if ($AllowUnsignedCommunityBeta) {
    foreach ($betaInput in @(
            [ordered]@{ path = $communityBetaDecisionPath; description = 'Community Beta decision record' },
            [ordered]@{ path = $communityBetaFfmpegArchive; description = 'Community Beta FFmpeg binary archive' },
            [ordered]@{ path = $communityBetaSourceBundle; description = 'Community Beta FFmpeg corresponding-source archive' }
        )) {
        $betaInputItem = Get-Item -LiteralPath $betaInput.path -Force
        if ($betaInputItem.PSIsContainer -or
            $betaInputItem.Length -le 0 -or
            ($betaInputItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "$($betaInput.description) must be a non-empty regular file without reparse points."
        }
    }
}
elseif ($AllowPersonalCommunityStable) {
    foreach ($personalInput in @(
            [ordered]@{ path = $personalCommunityStableDecisionPath; description = 'personal community stable decision record' },
            [ordered]@{ path = $personalCommunityStableFfmpegArchive; description = 'personal community stable FFmpeg binary archive' },
            [ordered]@{ path = $personalCommunityStableSourceBundle; description = 'personal community stable FFmpeg corresponding-source archive' }
        )) {
        $personalInputItem = Get-Item -LiteralPath $personalInput.path -Force
        if ($personalInputItem.PSIsContainer -or
            $personalInputItem.Length -le 0 -or
            ($personalInputItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "$($personalInput.description) must be a non-empty regular file without reparse points."
        }
    }
}

Assert-PeFile -LiteralPath $mainExecutable -Description 'main executable'
Assert-PeFile -LiteralPath $nsisBundle -Description 'NSIS installer'
if ([System.IO.Path]::GetExtension($nsisBundle) -ne '.exe') {
    throw "NSIS installer must use the .exe extension: '$nsisBundle'."
}
$nsisPeLayout = Get-PeLayoutReport -LiteralPath $nsisBundle -Description 'NSIS installer'
$nsisHeader = Get-NsisHeaderReport -LiteralPath $nsisBundle -PeLayout $nsisPeLayout

$manifest = Get-Content -Raw -LiteralPath $manifestPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
$schemaVersion = Get-RequiredJsonProperty -Object $manifest -Name 'schemaVersion' -Context 'FFmpeg manifest'
if ($schemaVersion -isnot [long] -and $schemaVersion -isnot [int]) {
    throw 'FFmpeg manifest schemaVersion must be an integer.'
}
if ([long] $schemaVersion -lt 1) {
    throw 'FFmpeg manifest schemaVersion must be at least 1.'
}

$platform = Get-RequiredJsonString -Object $manifest -Name 'platform' -Context 'FFmpeg manifest'
if (-not [string]::Equals($platform, 'windows', [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "FFmpeg manifest platform must be 'windows'; got '$platform'."
}
$architecture = Get-RequiredJsonString -Object $manifest -Name 'architecture' -Context 'FFmpeg manifest'
if ($architecture -notin @('x86_64', 'x64', 'amd64')) {
    throw "FFmpeg manifest architecture must be Windows x64; got '$architecture'."
}

$provider = Get-RequiredJsonProperty -Object $manifest -Name 'provider' -Context 'FFmpeg manifest'
[void] (Get-RequiredJsonString -Object $provider -Name 'name' -Context 'FFmpeg manifest provider')
$providerRepositoryUrl = Get-RequiredJsonString -Object $provider -Name 'repositoryUrl' -Context 'FFmpeg manifest provider'
Assert-HttpsUrl -Value $providerRepositoryUrl -Description 'FFmpeg provider repositoryUrl'
$providerReleaseTag = Get-RequiredJsonString -Object $provider -Name 'releaseTag' -Context 'FFmpeg manifest provider'

$artifact = Get-RequiredJsonProperty -Object $manifest -Name 'artifact' -Context 'FFmpeg manifest'
$artifactFileName = Get-RequiredJsonString -Object $artifact -Name 'fileName' -Context 'FFmpeg manifest artifact'
$artifactUrl = Get-RequiredJsonString -Object $artifact -Name 'url' -Context 'FFmpeg manifest artifact'
Assert-HttpsUrl -Value $artifactUrl -Description 'FFmpeg artifact URL'
$archiveSize = Get-RequiredJsonProperty -Object $artifact -Name 'sizeBytes' -Context 'FFmpeg manifest artifact'
if ([long] $archiveSize -le 0) {
    throw 'FFmpeg manifest artifact.sizeBytes must be positive.'
}
$archiveHash = Get-RequiredJsonString -Object $artifact -Name 'sha256' -Context 'FFmpeg manifest artifact'
Assert-Sha256Text -Value $archiveHash -Description 'FFmpeg archive hash'

$memberProperty = $artifact.PSObject.Properties['executableMember']
if ($null -eq $memberProperty) {
    $memberProperty = $artifact.PSObject.Properties['archiveMember']
}
if ($null -eq $memberProperty -or $memberProperty.Value -isnot [string] -or [string]::IsNullOrWhiteSpace($memberProperty.Value)) {
    throw "FFmpeg manifest artifact must provide a non-empty 'executableMember' or 'archiveMember'."
}
$archiveMember = [string] $memberProperty.Value
if ($archiveMember.Replace('\', '/') -notmatch '(^|/)bin/ffmpeg\.exe$') {
    throw "FFmpeg executable archive member must end in bin/ffmpeg.exe; got '$archiveMember'."
}

$expectedFfmpegSize = [long] (Get-RequiredJsonProperty -Object $artifact -Name 'executableSizeBytes' -Context 'FFmpeg manifest artifact')
if ($expectedFfmpegSize -le 0) {
    throw 'FFmpeg manifest artifact.executableSizeBytes must be positive.'
}
$expectedFfmpegHash = Get-RequiredJsonString -Object $artifact -Name 'executableSha256' -Context 'FFmpeg manifest artifact'
Assert-Sha256Text -Value $expectedFfmpegHash -Description 'FFmpeg executable hash'
$destination = Get-RequiredJsonString -Object $artifact -Name 'destination' -Context 'FFmpeg manifest artifact'
$normalizedDestination = $destination.Replace('\', '/').TrimStart([char[]] @('.', '/'))
if (-not ($normalizedDestination.EndsWith('resources/bin/ffmpeg.exe', [System.StringComparison]::OrdinalIgnoreCase) -or
        [string]::Equals($normalizedDestination, 'bin/ffmpeg.exe', [System.StringComparison]::OrdinalIgnoreCase))) {
    throw "FFmpeg manifest artifact.destination must target resources/bin/ffmpeg.exe; got '$destination'."
}

$ffmpegMetadata = Get-RequiredJsonProperty -Object $manifest -Name 'ffmpeg' -Context 'FFmpeg manifest'
$ffmpegVersion = Get-RequiredJsonString -Object $ffmpegMetadata -Name 'version' -Context 'FFmpeg manifest ffmpeg metadata'
$ffmpegLicenseExpression = Get-RequiredJsonString -Object $ffmpegMetadata -Name 'licenseExpression' -Context 'FFmpeg manifest ffmpeg metadata'
if ($ffmpegLicenseExpression -notmatch '^LGPL-') {
    throw "Bundled FFmpeg must use an LGPL license expression; got '$ffmpegLicenseExpression'."
}

$runtimeContract = Get-RequiredJsonProperty -Object $manifest -Name 'runtimeContract' -Context 'FFmpeg manifest'
$requiredCapabilities = Get-RequiredJsonProperty -Object $runtimeContract -Name 'requiredCapabilities' -Context 'FFmpeg runtimeContract'
foreach ($capabilityName in @('protocols', 'demuxers', 'decoders', 'filters', 'encoders', 'muxers')) {
    $capabilities = @(Get-RequiredJsonProperty -Object $requiredCapabilities -Name $capabilityName -Context 'FFmpeg runtimeContract.requiredCapabilities')
    if ($capabilities.Count -eq 0) {
        throw "FFmpeg runtime capability '$capabilityName' must not be empty."
    }
}
$thumbnailArguments = @(Get-RequiredJsonProperty -Object $runtimeContract -Name 'thumbnailArguments' -Context 'FFmpeg runtimeContract')
if ($thumbnailArguments.Count -eq 0) {
    throw 'FFmpeg runtimeContract.thumbnailArguments must not be empty.'
}
if ([long] (Get-RequiredJsonProperty -Object $runtimeContract -Name 'maximumOutputBytes' -Context 'FFmpeg runtimeContract') -le 0) {
    throw 'FFmpeg runtimeContract.maximumOutputBytes must be positive.'
}

$licenseProperty = $manifest.PSObject.Properties['licensePolicy']
if ($null -eq $licenseProperty) {
    $licenseProperty = $manifest.PSObject.Properties['license']
}
if ($null -eq $licenseProperty -or $null -eq $licenseProperty.Value) {
    throw "FFmpeg manifest must provide 'licensePolicy' (or the legacy-compatible 'license') metadata."
}
$licenseMetadata = $licenseProperty.Value
if (@($licenseMetadata.PSObject.Properties).Count -eq 0) {
    throw 'FFmpeg license metadata must not be empty.'
}

$sourceCompliance = Get-RequiredJsonProperty -Object $manifest -Name 'sourceCompliance' -Context 'FFmpeg manifest'
$redistributionReadyValue = Get-RequiredJsonProperty -Object $sourceCompliance -Name 'redistributionReady' -Context 'FFmpeg sourceCompliance'
if ($redistributionReadyValue -isnot [bool]) {
    throw 'FFmpeg sourceCompliance.redistributionReady must be a Boolean.'
}
$redistributionReady = [bool] $redistributionReadyValue
$sourceComplianceStatus = Get-RequiredJsonString -Object $sourceCompliance -Name 'status' -Context 'FFmpeg sourceCompliance'

if ($AllowUnsignedCommunityBeta) {
    $releaseChannel = Get-RequiredJsonString -Object $manifest -Name 'releaseChannel' -Context 'FFmpeg manifest'
    if ($releaseChannel -cne 'community-beta' -or
        $redistributionReady -or
        $sourceComplianceStatus -cne 'community-beta-source-bundled-formal-review-pending') {
        throw 'Community Beta FFmpeg manifest must remain beta-only and must not claim strict redistribution approval.'
    }

    $buildContract = Get-RequiredJsonProperty -Object $manifest -Name 'build' -Context 'Community Beta FFmpeg manifest'
    $externalLibraries = @(Get-RequiredJsonProperty -Object $buildContract -Name 'externalLibraries' -Context 'Community Beta FFmpeg build')
    if ($externalLibraries.Count -ne 0) {
        throw 'Community Beta FFmpeg must be the minimal build with no external libraries.'
    }

    $projectMirrorUrl = Get-RequiredJsonString -Object $artifact -Name 'projectMirrorUrl' -Context 'Community Beta FFmpeg artifact'
    $binaryMirrorUrl = Get-RequiredJsonString -Object $sourceCompliance -Name 'binaryMirrorUrl' -Context 'Community Beta FFmpeg sourceCompliance'
    Assert-HttpsUrl -Value $projectMirrorUrl -Description 'Community Beta FFmpeg artifact projectMirrorUrl'
    Assert-HttpsUrl -Value $binaryMirrorUrl -Description 'Community Beta FFmpeg sourceCompliance.binaryMirrorUrl'
    if ($artifactUrl -cne $projectMirrorUrl -or $artifactUrl -cne $binaryMirrorUrl) {
        throw 'Community Beta FFmpeg artifact and binary mirror URLs must match exactly.'
    }

    $artifactUri = [System.Uri] $artifactUrl
    $artifactUrlFileName = [System.Uri]::UnescapeDataString([System.IO.Path]::GetFileName($artifactUri.AbsolutePath))
    $binaryArchiveItem = Get-Item -LiteralPath $communityBetaFfmpegArchive -Force
    if ($artifactUrlFileName -cne $artifactFileName -or
        $binaryArchiveItem.Name -cne $artifactFileName -or
        $binaryArchiveItem.Length -ne [long] $archiveSize -or
        -not [string]::Equals((Get-Sha256 -LiteralPath $communityBetaFfmpegArchive), $archiveHash, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'Community Beta FFmpeg binary archive does not match its pinned manifest metadata.'
    }

    $sourceBundle = Get-RequiredJsonProperty -Object $sourceCompliance -Name 'correspondingSourceBundle' -Context 'Community Beta FFmpeg sourceCompliance'
    $sourceBundleUrl = Get-RequiredJsonString -Object $sourceBundle -Name 'url' -Context 'Community Beta FFmpeg correspondingSourceBundle'
    Assert-HttpsUrl -Value $sourceBundleUrl -Description 'Community Beta FFmpeg corresponding source URL'
    $sourceBundleSize = [long] (Get-RequiredJsonProperty -Object $sourceBundle -Name 'sizeBytes' -Context 'Community Beta FFmpeg correspondingSourceBundle')
    $sourceBundleHash = Get-RequiredJsonString -Object $sourceBundle -Name 'sha256' -Context 'Community Beta FFmpeg correspondingSourceBundle'
    Assert-Sha256Text -Value $sourceBundleHash -Description 'Community Beta FFmpeg corresponding source hash'
    $sourceBundleUri = [System.Uri] $sourceBundleUrl
    $sourceBundleUrlFileName = [System.Uri]::UnescapeDataString([System.IO.Path]::GetFileName($sourceBundleUri.AbsolutePath))
    $sourceBundleItem = Get-Item -LiteralPath $communityBetaSourceBundle -Force
    if ($sourceBundleUrlFileName -cne $sourceBundleItem.Name -or
        $sourceBundleItem.Length -ne $sourceBundleSize -or
        -not [string]::Equals((Get-Sha256 -LiteralPath $communityBetaSourceBundle), $sourceBundleHash, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'Community Beta FFmpeg corresponding-source archive does not match its pinned manifest metadata.'
    }

    foreach ($releaseUrl in @($artifactUri, $sourceBundleUri)) {
        if ($releaseUrl.AbsolutePath -notlike "*/releases/download/$providerReleaseTag/*") {
            throw "Community Beta FFmpeg release URL is not bound to tag '$providerReleaseTag'."
        }
    }
    $externalLibraryAuditComplete = Get-RequiredJsonProperty -Object $sourceCompliance -Name 'ffmpegExternalLibraryAuditComplete' -Context 'Community Beta FFmpeg sourceCompliance'
    $thirdPartyLicenseAuditComplete = Get-RequiredJsonProperty -Object $sourceCompliance -Name 'thirdPartyLicenseAuditComplete' -Context 'Community Beta FFmpeg sourceCompliance'
    $toolchainRuntimeLicenseReviewStatus = Get-RequiredJsonString -Object $sourceCompliance -Name 'toolchainRuntimeLicenseReviewStatus' -Context 'Community Beta FFmpeg sourceCompliance'
    $ijgAttributionRequired = Get-RequiredJsonProperty -Object $sourceCompliance -Name 'ijgAttributionRequired' -Context 'Community Beta FFmpeg sourceCompliance'
    $ijgAttributionIncluded = Get-RequiredJsonProperty -Object $sourceCompliance -Name 'ijgAttributionIncluded' -Context 'Community Beta FFmpeg sourceCompliance'
    $patentReviewStatus = Get-RequiredJsonString -Object $sourceCompliance -Name 'patentReviewStatus' -Context 'Community Beta FFmpeg sourceCompliance'
    if ($externalLibraryAuditComplete -isnot [bool] -or -not [bool] $externalLibraryAuditComplete -or
        $thirdPartyLicenseAuditComplete -isnot [bool] -or [bool] $thirdPartyLicenseAuditComplete -or
        $toolchainRuntimeLicenseReviewStatus -cne 'pending-for-strict-public-release' -or
        $ijgAttributionRequired -isnot [bool] -or -not [bool] $ijgAttributionRequired -or
        $ijgAttributionIncluded -isnot [bool] -or -not [bool] $ijgAttributionIncluded -or
        $patentReviewStatus -cne 'pending-for-strict-public-release') {
        throw 'Community Beta FFmpeg must prove the zero-external-library audit and IJG notice while retaining honest pending toolchain-runtime-license, third-party-license, and patent reviews.'
    }
    $legalApprovalProperty = $sourceCompliance.PSObject.Properties['legalApprovalReference']
    if ($null -eq $legalApprovalProperty -or $null -ne $legalApprovalProperty.Value) {
        throw 'Community Beta FFmpeg must not claim a formal legal approval reference.'
    }
}
elseif ($AllowPersonalCommunityStable) {
    $releaseChannel = Get-RequiredJsonString -Object $manifest -Name 'releaseChannel' -Context 'personal community stable FFmpeg manifest'
    $productionPromotionAuthorized = Get-RequiredJsonProperty -Object $manifest -Name 'productionPromotionAuthorized' -Context 'personal community stable FFmpeg manifest'
    $ownerAuthorized = Get-RequiredJsonProperty -Object $sourceCompliance -Name 'ownerAuthorizedForThisChannel' -Context 'personal community stable FFmpeg sourceCompliance'
    if ([long] $schemaVersion -ne 2 -or
        $releaseChannel -cne 'personal-community-stable' -or
        $productionPromotionAuthorized -isnot [bool] -or [bool] $productionPromotionAuthorized -or
        $ownerAuthorized -isnot [bool] -or -not [bool] $ownerAuthorized -or
        $redistributionReady -or
        $sourceComplianceStatus -cne 'personal-community-stable-source-bundled-owner-attested') {
        throw 'Personal community stable FFmpeg manifest must be owner-authorized for that channel while retaining strict public promotion and redistribution approval as false.'
    }
    if ($providerReleaseTag -cne $personalCommunityStableTag) {
        throw "Personal community stable FFmpeg provider tag '$providerReleaseTag' does not match owner decision tag '$personalCommunityStableTag'."
    }
    $manifestOwnerDecision = Get-RequiredJsonProperty -Object $manifest -Name 'ownerDecision' -Context 'personal community stable FFmpeg manifest'
    $expectedOwnerDecisionRelativePath = [System.IO.Path]::GetRelativePath($repositoryRoot, $personalCommunityStableDecisionPath).Replace('\', '/')
    $manifestOwnerDecisionHash = Get-RequiredJsonString -Object $manifestOwnerDecision -Name 'sha256' -Context 'personal community stable FFmpeg ownerDecision'
    Assert-Sha256Text -Value $manifestOwnerDecisionHash -Description 'personal community stable FFmpeg owner decision hash'
    if ((Get-RequiredJsonString -Object $manifestOwnerDecision -Name 'path' -Context 'personal community stable FFmpeg ownerDecision') -cne $expectedOwnerDecisionRelativePath -or
        -not [string]::Equals($manifestOwnerDecisionHash, (Get-Sha256 -LiteralPath $personalCommunityStableDecisionPath), [System.StringComparison]::OrdinalIgnoreCase) -or
        (Get-RequiredJsonString -Object $manifestOwnerDecision -Name 'version' -Context 'personal community stable FFmpeg ownerDecision') -cne $personalCommunityStableVersion -or
        (Get-RequiredJsonString -Object $manifestOwnerDecision -Name 'tag' -Context 'personal community stable FFmpeg ownerDecision') -cne $personalCommunityStableTag -or
        (Get-RequiredJsonString -Object $manifestOwnerDecision -Name 'decision' -Context 'personal community stable FFmpeg ownerDecision') -cne 'approved-by-repository-release-owner') {
        throw 'Personal community stable FFmpeg manifest ownerDecision does not match the exact approved repository decision.'
    }
    $providerReleaseUrl = Get-RequiredJsonString -Object $provider -Name 'releaseUrl' -Context 'personal community stable FFmpeg provider'
    Assert-HttpsUrl -Value $providerReleaseUrl -Description 'personal community stable FFmpeg provider releaseUrl'
    $providerReleaseUri = [System.Uri] $providerReleaseUrl
    if ($providerReleaseUri.AbsolutePath -notlike "*/releases/tag/$providerReleaseTag") {
        throw "Personal community stable FFmpeg provider release URL is not bound to tag '$providerReleaseTag'."
    }

    $buildContract = Get-RequiredJsonProperty -Object $manifest -Name 'build' -Context 'personal community stable FFmpeg manifest'
    $externalLibraries = @(Get-RequiredJsonProperty -Object $buildContract -Name 'externalLibraries' -Context 'personal community stable FFmpeg build')
    if ($externalLibraries.Count -ne 0) {
        throw 'Personal community stable FFmpeg must be the minimal build with no external libraries.'
    }

    $projectMirrorUrl = Get-RequiredJsonString -Object $artifact -Name 'projectMirrorUrl' -Context 'personal community stable FFmpeg artifact'
    $binaryMirrorUrl = Get-RequiredJsonString -Object $sourceCompliance -Name 'binaryMirrorUrl' -Context 'personal community stable FFmpeg sourceCompliance'
    Assert-HttpsUrl -Value $projectMirrorUrl -Description 'personal community stable FFmpeg artifact projectMirrorUrl'
    Assert-HttpsUrl -Value $binaryMirrorUrl -Description 'personal community stable FFmpeg sourceCompliance.binaryMirrorUrl'
    if ($artifactUrl -cne $projectMirrorUrl -or $artifactUrl -cne $binaryMirrorUrl) {
        throw 'Personal community stable FFmpeg artifact and binary mirror URLs must match exactly.'
    }

    $artifactUri = [System.Uri] $artifactUrl
    $artifactUrlFileName = [System.Uri]::UnescapeDataString([System.IO.Path]::GetFileName($artifactUri.AbsolutePath))
    $binaryArchiveItem = Get-Item -LiteralPath $personalCommunityStableFfmpegArchive -Force
    if ($artifactFileName -cne 'valoframe-ffmpeg-minimal-windows-x64.zip' -or
        $artifactUrlFileName -cne $artifactFileName -or
        $binaryArchiveItem.Name -cne $artifactFileName -or
        $binaryArchiveItem.Length -ne [long] $archiveSize -or
        -not [string]::Equals((Get-Sha256 -LiteralPath $personalCommunityStableFfmpegArchive), $archiveHash, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'Personal community stable FFmpeg binary archive does not match its pinned minimal-build manifest metadata.'
    }

    $sourceBundle = Get-RequiredJsonProperty -Object $sourceCompliance -Name 'correspondingSourceBundle' -Context 'personal community stable FFmpeg sourceCompliance'
    $sourceBundleUrl = Get-RequiredJsonString -Object $sourceBundle -Name 'url' -Context 'personal community stable FFmpeg correspondingSourceBundle'
    Assert-HttpsUrl -Value $sourceBundleUrl -Description 'personal community stable FFmpeg corresponding source URL'
    $sourceBundleSize = [long] (Get-RequiredJsonProperty -Object $sourceBundle -Name 'sizeBytes' -Context 'personal community stable FFmpeg correspondingSourceBundle')
    if ($sourceBundleSize -le 0) {
        throw 'Personal community stable FFmpeg corresponding source sizeBytes must be positive.'
    }
    $sourceBundleHash = Get-RequiredJsonString -Object $sourceBundle -Name 'sha256' -Context 'personal community stable FFmpeg correspondingSourceBundle'
    Assert-Sha256Text -Value $sourceBundleHash -Description 'personal community stable FFmpeg corresponding source hash'
    $sourceBundleUri = [System.Uri] $sourceBundleUrl
    $sourceBundleUrlFileName = [System.Uri]::UnescapeDataString([System.IO.Path]::GetFileName($sourceBundleUri.AbsolutePath))
    $sourceBundleItem = Get-Item -LiteralPath $personalCommunityStableSourceBundle -Force
    if ($sourceBundleUrlFileName -cne 'ffmpeg-corresponding-source.tar.xz' -or
        $sourceBundleItem.Name -cne $sourceBundleUrlFileName -or
        $sourceBundleItem.Length -ne $sourceBundleSize -or
        -not [string]::Equals((Get-Sha256 -LiteralPath $personalCommunityStableSourceBundle), $sourceBundleHash, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'Personal community stable FFmpeg corresponding-source archive does not match its pinned manifest metadata.'
    }

    foreach ($releaseUrl in @($artifactUri, $sourceBundleUri)) {
        if ($releaseUrl.AbsolutePath -notlike "*/releases/download/$providerReleaseTag/*") {
            throw "Personal community stable FFmpeg release URL is not bound to tag '$providerReleaseTag'."
        }
    }

    $externalLibraryAuditComplete = Get-RequiredJsonProperty -Object $sourceCompliance -Name 'ffmpegExternalLibraryAuditComplete' -Context 'personal community stable FFmpeg sourceCompliance'
    $thirdPartyLicenseAuditComplete = Get-RequiredJsonProperty -Object $sourceCompliance -Name 'thirdPartyLicenseAuditComplete' -Context 'personal community stable FFmpeg sourceCompliance'
    $toolchainRuntimeLicenseReviewStatus = Get-RequiredJsonString -Object $sourceCompliance -Name 'toolchainRuntimeLicenseReviewStatus' -Context 'personal community stable FFmpeg sourceCompliance'
    $ijgAttributionRequired = Get-RequiredJsonProperty -Object $sourceCompliance -Name 'ijgAttributionRequired' -Context 'personal community stable FFmpeg sourceCompliance'
    $ijgAttributionIncluded = Get-RequiredJsonProperty -Object $sourceCompliance -Name 'ijgAttributionIncluded' -Context 'personal community stable FFmpeg sourceCompliance'
    $patentReviewStatus = Get-RequiredJsonString -Object $sourceCompliance -Name 'patentReviewStatus' -Context 'personal community stable FFmpeg sourceCompliance'
    if ($externalLibraryAuditComplete -isnot [bool] -or -not [bool] $externalLibraryAuditComplete -or
        $thirdPartyLicenseAuditComplete -isnot [bool] -or [bool] $thirdPartyLicenseAuditComplete -or
        $toolchainRuntimeLicenseReviewStatus -cne 'pending-for-strict-public-release' -or
        $ijgAttributionRequired -isnot [bool] -or -not [bool] $ijgAttributionRequired -or
        $ijgAttributionIncluded -isnot [bool] -or -not [bool] $ijgAttributionIncluded -or
        $patentReviewStatus -cne 'pending-for-strict-public-release') {
        throw 'Personal community stable FFmpeg must prove the zero-external-library audit and IJG notice while retaining honest pending strict-public toolchain, third-party-license, and patent reviews.'
    }
    $legalApprovalProperty = $sourceCompliance.PSObject.Properties['legalApprovalReference']
    if ($null -eq $legalApprovalProperty -or $null -ne $legalApprovalProperty.Value) {
        throw 'Personal community stable FFmpeg must not claim a formal legal approval reference.'
    }
}
elseif (-not $AllowUnsignedInternalRc) {
    if (-not $redistributionReady) {
        throw 'Public redistribution is blocked: FFmpeg sourceCompliance.redistributionReady is false. Use -AllowUnsignedInternalRc only for a non-redistributed internal RC.'
    }
    if ($sourceComplianceStatus -cne 'ready-for-redistribution') {
        throw "Public redistribution requires FFmpeg sourceCompliance.status to be exactly 'ready-for-redistribution'; got '$sourceComplianceStatus'."
    }

    $projectMirrorUrl = Get-RequiredJsonString -Object $artifact -Name 'projectMirrorUrl' -Context 'FFmpeg manifest artifact'
    Assert-HttpsUrl -Value $projectMirrorUrl -Description 'FFmpeg artifact projectMirrorUrl'
    $binaryMirrorUrl = Get-RequiredJsonString -Object $sourceCompliance -Name 'binaryMirrorUrl' -Context 'FFmpeg sourceCompliance'
    Assert-HttpsUrl -Value $binaryMirrorUrl -Description 'FFmpeg sourceCompliance.binaryMirrorUrl'
    if (-not [string]::Equals($projectMirrorUrl, $binaryMirrorUrl, [System.StringComparison]::Ordinal)) {
        throw 'Public redistribution requires artifact.projectMirrorUrl and sourceCompliance.binaryMirrorUrl to match exactly.'
    }

    foreach ($booleanGate in @('thirdPartyLicenseAuditComplete', 'ijgAttributionIncluded')) {
        $gate = Get-RequiredJsonProperty -Object $sourceCompliance -Name $booleanGate -Context 'FFmpeg sourceCompliance'
        if ($gate -isnot [bool] -or -not [bool] $gate) {
            throw "Public redistribution requires FFmpeg sourceCompliance.$booleanGate to be true."
        }
    }

    $sourceBundle = Get-RequiredJsonProperty -Object $sourceCompliance -Name 'correspondingSourceBundle' -Context 'FFmpeg sourceCompliance'
    $sourceBundleUrl = Get-RequiredJsonString -Object $sourceBundle -Name 'url' -Context 'FFmpeg correspondingSourceBundle'
    Assert-HttpsUrl -Value $sourceBundleUrl -Description 'FFmpeg corresponding source bundle URL'
    if ([long] (Get-RequiredJsonProperty -Object $sourceBundle -Name 'sizeBytes' -Context 'FFmpeg correspondingSourceBundle') -le 0) {
        throw 'FFmpeg corresponding source bundle sizeBytes must be positive.'
    }
    $sourceBundleHash = Get-RequiredJsonString -Object $sourceBundle -Name 'sha256' -Context 'FFmpeg correspondingSourceBundle'
    Assert-Sha256Text -Value $sourceBundleHash -Description 'FFmpeg corresponding source bundle hash'

    $patentStatus = Get-RequiredJsonString -Object $sourceCompliance -Name 'patentReviewStatus' -Context 'FFmpeg sourceCompliance'
    if ($patentStatus -cnotin @('approved', 'not-required')) {
        throw "Public redistribution requires patentReviewStatus to be exactly 'approved' or 'not-required'; got '$patentStatus'."
    }
    [void] (Get-RequiredJsonString -Object $sourceCompliance -Name 'legalApprovalReference' -Context 'FFmpeg sourceCompliance')
}
elseif (-not $redistributionReady) {
    Write-Warning 'FFmpeg public redistribution metadata is incomplete; accepted only for this explicitly internal RC check.'
}

$ffmpegPath = Get-ResourceFile -Root $resourceRoot -RelativePath $FfmpegRelativePath -Description 'bundled FFmpeg executable'
Assert-PeFile -LiteralPath $ffmpegPath -Description 'bundled FFmpeg executable'
$ffmpegItem = Get-Item -LiteralPath $ffmpegPath -Force
$ffmpegHash = Get-Sha256 -LiteralPath $ffmpegPath
if ($ffmpegItem.Length -ne $expectedFfmpegSize) {
    throw "Bundled FFmpeg size mismatch. Expected $expectedFfmpegSize bytes, got $($ffmpegItem.Length)."
}
if (-not [string]::Equals($ffmpegHash, $expectedFfmpegHash, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Bundled FFmpeg SHA-256 mismatch. Expected '$expectedFfmpegHash', got '$ffmpegHash'."
}

$licenseDeclarations = [System.Collections.Generic.Dictionary[string, object]]::new(
    [System.StringComparer]::OrdinalIgnoreCase
)
$licenseDeclarations[$projectLicenseRelativePath] = $null
$licenseDeclarations[$projectLicenseScopeRelativePath] = $null
foreach ($relativePath in $LicenseRelativePaths) {
    if ([string]::IsNullOrWhiteSpace($relativePath)) {
        throw 'LicenseRelativePaths must not contain empty values.'
    }
    $licenseDeclarations[$relativePath] = $null
}

$manifestLicenseFilesProperty = $licenseMetadata.PSObject.Properties['files']
if ($null -ne $manifestLicenseFilesProperty -and $null -ne $manifestLicenseFilesProperty.Value) {
    foreach ($fileDeclaration in @($manifestLicenseFilesProperty.Value)) {
        $declaredPath = Get-RequiredJsonString -Object $fileDeclaration -Name 'path' -Context 'FFmpeg license file declaration'
        $normalizedDeclaredPath = $declaredPath.Replace('\', '/')
        $prefix = 'src-tauri/resources/'
        if (-not $normalizedDeclaredPath.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "FFmpeg license file declaration must be below src-tauri/resources: '$declaredPath'."
        }
        $relative = $normalizedDeclaredPath.Substring($prefix.Length).Replace('/', '\')
        $licenseDeclarations[$relative] = $fileDeclaration
    }
}
if ($AllowPersonalCommunityStable -and
    -not $licenseDeclarations.ContainsKey('licenses\ffmpeg\THIRD-PARTY-NOTICE.md')) {
    throw 'Personal community stable FFmpeg manifest must declare THIRD-PARTY-NOTICE.md as an installed license payload.'
}

$licenseReports = [System.Collections.Generic.List[object]]::new()
foreach ($declaration in $licenseDeclarations.GetEnumerator()) {
    $licensePath = Get-ResourceFile -Root $resourceRoot -RelativePath $declaration.Key -Description "bundled license file '$($declaration.Key)'"
    $licenseItem = Get-Item -LiteralPath $licensePath -Force
    $licenseHash = Get-Sha256 -LiteralPath $licensePath
    if ($null -ne $declaration.Value) {
        $declaredSizeProperty = $declaration.Value.PSObject.Properties['sizeBytes']
        if ($null -ne $declaredSizeProperty -and $null -ne $declaredSizeProperty.Value -and
            $licenseItem.Length -ne [long] $declaredSizeProperty.Value) {
            throw "License file '$($declaration.Key)' size does not match the FFmpeg manifest."
        }
        $declaredHashProperty = $declaration.Value.PSObject.Properties['sha256']
        if ($null -ne $declaredHashProperty -and $null -ne $declaredHashProperty.Value) {
            $declaredHash = [string] $declaredHashProperty.Value
            Assert-Sha256Text -Value $declaredHash -Description "License file '$($declaration.Key)' manifest hash"
            if (-not [string]::Equals($licenseHash, $declaredHash, [System.StringComparison]::OrdinalIgnoreCase)) {
                throw "License file '$($declaration.Key)' SHA-256 does not match the FFmpeg manifest."
            }
        }
    }
    $licenseReports.Add([ordered]@{
            relativePath = $declaration.Key.Replace('\', '/')
            sizeBytes = $licenseItem.Length
            sha256 = $licenseHash
        })
}

$bundledProjectLicensePath = Get-ResourceFile -Root $resourceRoot -RelativePath $projectLicenseRelativePath -Description 'bundled project MIT license'
$projectLicenseItem = Get-Item -LiteralPath $projectLicense -Force
$bundledProjectLicenseItem = Get-Item -LiteralPath $bundledProjectLicensePath -Force
$projectLicenseHash = Get-Sha256 -LiteralPath $projectLicense
$bundledProjectLicenseHash = Get-Sha256 -LiteralPath $bundledProjectLicensePath
if ($projectLicenseItem.Length -ne $bundledProjectLicenseItem.Length -or
    -not [string]::Equals($projectLicenseHash, $bundledProjectLicenseHash, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw 'Bundled project MIT license must exactly match the repository root LICENSE.'
}
$projectLicenseText = Get-Content -Raw -LiteralPath $bundledProjectLicensePath -Encoding UTF8
if ($projectLicenseText -notmatch '(?m)^MIT License\r?$' -or
    $projectLicenseText -notmatch 'Permission is hereby granted, free of charge' -or
    $projectLicenseText -notmatch 'Copyright \(c\) 2026 VALOFRAME Contributors') {
    throw 'Bundled project license does not contain the approved MIT license markers.'
}
$bundledProjectLicenseScopePath = Get-ResourceFile -Root $resourceRoot -RelativePath $projectLicenseScopeRelativePath -Description 'bundled project license scope'
$projectLicenseScopeItem = Get-Item -LiteralPath $projectLicenseScope -Force
$bundledProjectLicenseScopeItem = Get-Item -LiteralPath $bundledProjectLicenseScopePath -Force
$projectLicenseScopeHash = Get-Sha256 -LiteralPath $projectLicenseScope
$bundledProjectLicenseScopeHash = Get-Sha256 -LiteralPath $bundledProjectLicenseScopePath
if ($projectLicenseScopeItem.Length -ne $bundledProjectLicenseScopeItem.Length -or
    -not [string]::Equals($projectLicenseScopeHash, $bundledProjectLicenseScopeHash, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw 'Bundled project license scope must exactly match the repository root LICENSE-SCOPE.txt.'
}
$projectLicenseScopeText = Get-Content -Raw -LiteralPath $bundledProjectLicenseScopePath -Encoding UTF8
if ($projectLicenseScopeText -notmatch '(?m)^VALOFRAME LICENSING SCOPE\r?$' -or
    $projectLicenseScopeText -notmatch 'are not licensed under the\s+MIT License' -or
    $projectLicenseScopeText -notmatch 'third-party material') {
    throw 'Bundled project license scope does not contain the approved exclusion markers.'
}
$releasePolicy = Get-Content -Raw -LiteralPath $releasePolicyPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
$projectLicensePolicy = Get-RequiredJsonProperty -Object $releasePolicy -Name 'projectLicense' -Context 'public release policy'
$projectLicenseApproved = Get-RequiredJsonProperty -Object $projectLicensePolicy -Name 'approved' -Context 'public release project license'
if ($projectLicenseApproved -isnot [bool] -or -not $projectLicenseApproved) {
    throw 'Public release project license must be explicitly approved.'
}
if ((Get-RequiredJsonString -Object $projectLicensePolicy -Name 'spdxExpression' -Context 'public release project license') -cne 'MIT' -or
    (Get-RequiredJsonString -Object $projectLicensePolicy -Name 'file' -Context 'public release project license') -cne 'LICENSE' -or
    (Get-RequiredJsonString -Object $projectLicensePolicy -Name 'scopeFile' -Context 'public release project license') -cne 'LICENSE-SCOPE.txt') {
    throw 'Public release project license paths or SPDX expression are not the approved MIT values.'
}
$approvedProjectLicenseHash = Get-RequiredJsonString -Object $projectLicensePolicy -Name 'sha256' -Context 'public release project license'
$approvedProjectLicenseScopeHash = Get-RequiredJsonString -Object $projectLicensePolicy -Name 'scopeSha256' -Context 'public release project license'
Assert-Sha256Text -Value $approvedProjectLicenseHash -Description 'approved project MIT license hash'
Assert-Sha256Text -Value $approvedProjectLicenseScopeHash -Description 'approved project license scope hash'
if (-not [string]::Equals($projectLicenseHash, $approvedProjectLicenseHash, [System.StringComparison]::OrdinalIgnoreCase) -or
    -not [string]::Equals($projectLicenseScopeHash, $approvedProjectLicenseScopeHash, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw 'Project license or scope bytes do not match the approved public release policy SHA-256.'
}

$copyingPath = Get-ResourceFile -Root $resourceRoot -RelativePath 'licenses\ffmpeg\COPYING.LGPLv3.txt' -Description 'bundled LGPL license'
$copyingText = Get-Content -Raw -LiteralPath $copyingPath -Encoding UTF8
if ($copyingText -notmatch '(?i)GNU LESSER GENERAL PUBLIC LICENSE' -or $copyingText -notmatch '(?i)Version 3') {
    throw 'Bundled COPYING.LGPLv3.txt does not contain the expected LGPL version 3 wording.'
}

$buildInfoPath = Get-ResourceFile -Root $resourceRoot -RelativePath 'licenses\ffmpeg\BUILD-INFO.json' -Description 'bundled FFmpeg build metadata'
$buildInfo = Get-Content -Raw -LiteralPath $buildInfoPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
$buildInfoExecutable = Get-RequiredJsonProperty -Object $buildInfo -Name 'executable' -Context 'bundled FFmpeg BUILD-INFO.json'
$buildInfoHash = Get-RequiredJsonString -Object $buildInfoExecutable -Name 'sha256' -Context 'bundled FFmpeg BUILD-INFO executable'
Assert-Sha256Text -Value $buildInfoHash -Description 'bundled FFmpeg BUILD-INFO executable hash'
if (-not [string]::Equals($buildInfoHash, $ffmpegHash, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw 'Bundled FFmpeg BUILD-INFO executable hash does not match bin/ffmpeg.exe.'
}
if ([long] (Get-RequiredJsonProperty -Object $buildInfoExecutable -Name 'sizeBytes' -Context 'bundled FFmpeg BUILD-INFO executable') -ne $ffmpegItem.Length) {
    throw 'Bundled FFmpeg BUILD-INFO executable size does not match bin/ffmpeg.exe.'
}
$buildInfoVersion = Get-RequiredJsonString -Object $buildInfo -Name 'version' -Context 'bundled FFmpeg BUILD-INFO.json'
if (-not [string]::Equals($buildInfoVersion, $ffmpegVersion, [System.StringComparison]::Ordinal)) {
    throw 'Bundled FFmpeg BUILD-INFO version does not match the provenance manifest.'
}
$buildInfoLicense = Get-RequiredJsonString -Object $buildInfo -Name 'licenseExpression' -Context 'bundled FFmpeg BUILD-INFO.json'
if (-not [string]::Equals($buildInfoLicense, $ffmpegLicenseExpression, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw 'Bundled FFmpeg BUILD-INFO license expression does not match the provenance manifest.'
}
$buildRedistributionStatus = Get-RequiredJsonString -Object $buildInfo -Name 'redistributionStatus' -Context 'bundled FFmpeg BUILD-INFO.json'
if (-not [string]::Equals($buildRedistributionStatus, $sourceComplianceStatus, [System.StringComparison]::Ordinal)) {
    throw "Bundled FFmpeg BUILD-INFO redistributionStatus '$buildRedistributionStatus' does not exactly match manifest sourceCompliance.status '$sourceComplianceStatus'."
}

$sourceOfferPath = Get-ResourceFile -Root $resourceRoot -RelativePath 'licenses\ffmpeg\SOURCE-OFFER.md' -Description 'bundled FFmpeg source offer'
$sourceOfferText = Get-Content -Raw -LiteralPath $sourceOfferPath -Encoding UTF8
if ($sourceOfferText -notmatch 'https://[^\s)>]+') {
    throw 'Bundled SOURCE-OFFER.md must contain at least one HTTPS source URL.'
}
if ($AllowPersonalCommunityStable) {
    if ($sourceOfferText.IndexOf($sourceBundleUrl, [System.StringComparison]::Ordinal) -lt 0 -or
        $sourceOfferText.IndexOf($sourceBundleHash, [System.StringComparison]::OrdinalIgnoreCase) -lt 0) {
        throw 'Personal community stable SOURCE-OFFER.md must name the exact corresponding-source URL and SHA-256.'
    }
    $thirdPartyNoticePath = Get-ResourceFile -Root $resourceRoot -RelativePath 'licenses\ffmpeg\THIRD-PARTY-NOTICE.md' -Description 'bundled personal community stable FFmpeg third-party notice'
    $thirdPartyNoticeText = Get-Content -Raw -LiteralPath $thirdPartyNoticePath -Encoding UTF8
    if ($thirdPartyNoticeText -notmatch 'This software is based in part on the work of the Independent JPEG Group\.' -or
        $thirdPartyNoticeText.IndexOf($sourceBundleUrl, [System.StringComparison]::Ordinal) -lt 0 -or
        $thirdPartyNoticeText.IndexOf($sourceBundleHash, [System.StringComparison]::OrdinalIgnoreCase) -lt 0) {
        throw 'Personal community stable THIRD-PARTY-NOTICE.md must contain the IJG attribution and exact corresponding-source reference.'
    }
}

$complianceReport = Get-CompliancePayloadReports `
    -ResourceRoot $resourceRoot `
    -RelativeRoot $ThirdPartyComplianceRelativeRoot `
    -RepositoryRoot $repositoryRoot `
    -FfmpegManifestPath $manifestPath `
    -ExpectedReleaseProfile $(if ($AllowUnsignedCommunityBeta) {
        'community-beta'
    }
    elseif ($AllowPersonalCommunityStable) {
        'personal-community-stable'
    }
    else {
        'public'
    }) `
    -RequirePublicReady (-not ([bool] $AllowUnsignedInternalRc -or [bool] $AllowUnsignedCommunityBeta -or [bool] $AllowPersonalCommunityStable))

$mainHash = Get-Sha256 -LiteralPath $mainExecutable
$nsisHash = Get-Sha256 -LiteralPath $nsisBundle
Assert-ExpectedHash -Actual $mainHash -Expected $ExpectedMainExecutableSha256 -Description 'main executable'
Assert-ExpectedHash -Actual $nsisHash -Expected $ExpectedNsisBundleSha256 -Description 'NSIS installer'

$expectedPayload = [System.Collections.Generic.List[object]]::new()
$mainItem = Get-Item -LiteralPath $mainExecutable -Force
$expectedPayload.Add([ordered]@{
        destination = [System.IO.Path]::GetFileName($mainExecutable)
        sourcePath = $mainExecutable
        sizeBytes = $mainItem.Length
        sha256 = $mainHash
        comparison = 'tauri-nsis-marker-normalized'
    })
$expectedPayload.Add([ordered]@{
        destination = 'bin\ffmpeg.exe'
        sourcePath = $ffmpegPath
        sizeBytes = $ffmpegItem.Length
        sha256 = $ffmpegHash
    })
foreach ($licenseReport in $licenseReports) {
    $relativePath = ([string] $licenseReport.relativePath).Replace('/', '\')
    $payloadSource = Get-ResourceFile -Root $resourceRoot -RelativePath $relativePath -Description "required license payload '$relativePath'"
    $expectedPayload.Add([ordered]@{
            destination = $relativePath
            sourcePath = $payloadSource
            sizeBytes = [long] $licenseReport.sizeBytes
            sha256 = [string] $licenseReport.sha256
        })
}
foreach ($complianceEntry in @($complianceReport.entries)) {
    $expectedPayload.Add([ordered]@{
            destination = [string] $complianceEntry.destination
            sourcePath = [string] $complianceEntry.sourcePath
            sizeBytes = [long] $complianceEntry.sizeBytes
            sha256 = [string] $complianceEntry.sha256
        })
}

$nsisScriptPayload = Get-NsisScriptPayloadReport `
    -ScriptPath $nsisScript `
    -MainExecutable $mainExecutable `
    -ExpectedPayload $expectedPayload.ToArray()
$nsisExtractor = Resolve-NsisExtractor -ConfiguredPath $NsisExtractorPath
$extractorItem = Get-Item -LiteralPath $nsisExtractor -Force
if (($extractorItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw "NSIS extractor must not be a reparse point: '$nsisExtractor'."
}
$permitUnsigned = [bool] ($AllowUnsignedInternalRc -or $AllowUnsignedCommunityBeta -or $AllowPersonalCommunityStable)
$signingRequirements = if ($permitUnsigned) {
    $null
}
else {
    try {
        Get-PublicSigningRequirements `
            -RepositoryRoot $repositoryRoot `
            -PolicyPath $PublicReleasePolicyPath `
            -ConfiguredSigntoolPath $SigntoolPath
    }
    catch {
        throw "PUBLIC_SIGNING_POLICY_BLOCKED: $($_.Exception.Message)"
    }
}
$nsisArchivePayload = Assert-NsisArchivePayload `
    -ExtractorPath $nsisExtractor `
    -InstallerPath $nsisBundle `
    -NsisHeader $nsisHeader `
    -ExpectedPayload $expectedPayload.ToArray() `
    -PermitUnsignedApplicationArtifacts $permitUnsigned `
    -SigningRequirements $signingRequirements `
    -VerifiedOutputConfiguration $verifiedPayloadOutput

$mainSignature = Get-SignatureReport `
    -LiteralPath $mainExecutable `
    -Description 'external UNK staging main executable' `
    -PermitUnsigned $true `
    -UnsignedAcceptanceReason $(if ($AllowUnsignedInternalRc) {
        '-AllowUnsignedInternalRc was supplied'
    }
    elseif ($AllowUnsignedCommunityBeta) {
        '-AllowUnsignedCommunityBeta was supplied; the installer must be disclosed as unsigned'
    }
    elseif ($AllowPersonalCommunityStable) {
        '-AllowPersonalCommunityStable was supplied; NotSigned artifacts must be disclosed for this owner-authorized channel'
    }
    else {
        'the public artifact signature is verified on the embedded NSS executable'
    })
if (-not $permitUnsigned -and $mainSignature.status -cne 'NotSigned') {
    throw "Public release external UNK staging main executable must be NotSigned; got '$($mainSignature.status)'."
}
$nsisSignature = Get-SignatureReport `
    -LiteralPath $nsisBundle `
    -Description 'NSIS installer' `
    -PermitUnsigned $permitUnsigned `
    -SigningRequirements $signingRequirements
$ffmpegSignature = if ($AllowPersonalCommunityStable) {
    Get-SignatureReport `
        -LiteralPath $ffmpegPath `
        -Description 'bundled personal community stable FFmpeg executable' `
        -PermitUnsigned $true `
        -UnsignedAcceptanceReason '-AllowPersonalCommunityStable was supplied and the FFmpeg bytes are pinned to the owner-authorized minimal archive'
}
else {
    Get-SignatureReport -LiteralPath $ffmpegPath -Description 'bundled FFmpeg executable' -PermitUnsigned $true -HashPinnedOnly
}
if ($AllowUnsignedCommunityBeta -and
    ($mainSignature.status -cne 'NotSigned' -or
        $nsisSignature.status -cne 'NotSigned' -or
        [string] $nsisArchivePayload.embeddedMainSignature.status -cne 'NotSigned')) {
    throw 'Community Beta requires the staging executable, embedded executable, and installer to all be NotSigned so the warning is accurate.'
}

$report = [ordered]@{
    status = 'passed'
    releaseMode = if ($AllowUnsignedInternalRc) {
        'internal-rc'
    }
    elseif ($AllowUnsignedCommunityBeta) {
        'unsigned-community-beta'
    }
    elseif ($AllowPersonalCommunityStable) {
        'personal-community-stable'
    }
    else {
        'public-redistribution'
    }
    checkedAtUtc = [DateTime]::UtcNow.ToString('o')
    strictPublicReleaseApproved = -not [bool] ($AllowUnsignedInternalRc -or $AllowUnsignedCommunityBeta -or $AllowPersonalCommunityStable)
    communityBetaDecision = if ($AllowUnsignedCommunityBeta) {
        [ordered]@{
            path = $communityBetaDecisionPath
            sha256 = Get-Sha256 -LiteralPath $communityBetaDecisionPath
            strictPublicReleaseApproval = $false
            ffmpegBinaryArchive = [ordered]@{
                path = $communityBetaFfmpegArchive
                sizeBytes = (Get-Item -LiteralPath $communityBetaFfmpegArchive -Force).Length
                sha256 = Get-Sha256 -LiteralPath $communityBetaFfmpegArchive
            }
            ffmpegCorrespondingSource = [ordered]@{
                path = $communityBetaSourceBundle
                sizeBytes = (Get-Item -LiteralPath $communityBetaSourceBundle -Force).Length
                sha256 = Get-Sha256 -LiteralPath $communityBetaSourceBundle
            }
        }
    }
    else {
        $null
    }
    personalCommunityStableDecision = if ($AllowPersonalCommunityStable) {
        [ordered]@{
            path = $personalCommunityStableDecisionPath
            sha256 = Get-Sha256 -LiteralPath $personalCommunityStableDecisionPath
            version = $personalCommunityStableVersion
            tag = $personalCommunityStableTag
            channel = 'personal-community-stable'
            ownerAuthorizedForThisChannel = $true
            strictPublicReleaseApproval = $false
            ffmpegBinaryArchive = [ordered]@{
                path = $personalCommunityStableFfmpegArchive
                sizeBytes = (Get-Item -LiteralPath $personalCommunityStableFfmpegArchive -Force).Length
                sha256 = Get-Sha256 -LiteralPath $personalCommunityStableFfmpegArchive
            }
            ffmpegCorrespondingSource = [ordered]@{
                path = $personalCommunityStableSourceBundle
                sizeBytes = (Get-Item -LiteralPath $personalCommunityStableSourceBundle -Force).Length
                sha256 = Get-Sha256 -LiteralPath $personalCommunityStableSourceBundle
            }
        }
    }
    else {
        $null
    }
    publicSigningPolicy = if ($null -eq $signingRequirements) {
        $null
    }
    else {
        [ordered]@{
            path = [string] $signingRequirements.policyPath
            sha256 = [string] $signingRequirements.policySha256
            expectedPublisherSubject = [string] $signingRequirements.expectedPublisherSubject
            expectedCertificateThumbprint = [string] $signingRequirements.expectedCertificateThumbprint
            timestampUrl = [string] $signingRequirements.timestampUrl
            signtoolPath = [string] $signingRequirements.signtoolPath
            signtoolSha256 = [string] $signingRequirements.signtoolSha256
        }
    }
    artifacts = [ordered]@{
        mainExecutable = [ordered]@{
            path = $mainExecutable
            role = 'external-unk-staging'
            sizeBytes = (Get-Item -LiteralPath $mainExecutable).Length
            sha256 = $mainHash
            signature = $mainSignature
        }
        nsisInstaller = [ordered]@{
            path = $nsisBundle
            sizeBytes = (Get-Item -LiteralPath $nsisBundle).Length
            sha256 = $nsisHash
            signature = $nsisSignature
            format = [ordered]@{
                type = 'Nsis'
                subtype = 'NSIS-3 Unicode'
                peOverlayOffset = [long] $nsisHeader.overlayOffset
                firstHeaderLength = [long] $nsisHeader.headerLength
                followingDataLength = [long] $nsisHeader.followingDataLength
                authenticodeBytes = [long] $nsisHeader.authenticodeBytes
            }
        }
        ffmpeg = [ordered]@{
            path = $ffmpegPath
            sizeBytes = $ffmpegItem.Length
            sha256 = $ffmpegHash
            signature = $ffmpegSignature
        }
    }
    ffmpegManifest = [ordered]@{
        path = $manifestPath
        sha256 = Get-Sha256 -LiteralPath $manifestPath
        schemaVersion = [long] $schemaVersion
        providerReleaseTag = $providerReleaseTag
        archiveFileName = $artifactFileName
        archiveSha256 = $archiveHash.ToLowerInvariant()
        ffmpegVersion = $ffmpegVersion
        redistributionReady = $redistributionReady
        redistributionStatus = $sourceComplianceStatus
    }
    nsisScript = $nsisScriptPayload
    nsisPayload = [ordered]@{
        proof = 'controlled-static-extraction'
        boundary = 'Does not execute the installer or prove install, upgrade, uninstall, WebView2 bootstrap, or runtime behavior.'
        extractorPath = $nsisArchivePayload.extractorPath
        extractorSha256 = $nsisArchivePayload.extractorSha256
        archiveType = $nsisArchivePayload.archiveType
        archiveSubType = $nsisArchivePayload.archiveSubType
        verifiedPayloadOutputDirectory = $nsisArchivePayload.verifiedPayloadOutputDirectory
        verifiedPayloadOutputVerification = $nsisArchivePayload.verifiedPayloadOutputVerification
        embeddedMainSignature = $nsisArchivePayload.embeddedMainSignature
        entries = $nsisArchivePayload.entries
    }
    licenses = $licenseReports.ToArray()
    thirdPartyCompliance = $complianceReport
}

$report | ConvertTo-Json -Depth 12
