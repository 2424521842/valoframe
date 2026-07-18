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
listed and only the six expected application payload files are extracted into a
fresh temporary directory for hash comparison; the installer is never run.

.PARAMETER VerifiedPayloadOutputDirectory
Optional pre-created empty directory below the caller's temporary directory.
After every payload check passes, the six controlled-extraction files are moved
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

    [string] $ExpectedMainExecutableSha256,

    [string] $ExpectedNsisBundleSha256,

    [switch] $AllowUnsignedInternalRc
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

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
    return [ordered]@{
        status = $status
        statusMessage = [string] $signature.StatusMessage
        signerSubject = if ($null -eq $certificate) { $null } else { $certificate.Subject }
        signerThumbprint = if ($null -eq $certificate) { $null } else { $certificate.Thumbprint }
        signerNotAfterUtc = if ($null -eq $certificate) { $null } else { $certificate.NotAfter.ToUniversalTime().ToString('o') }
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
    if ($files.Count -ne $EntryReports.Count -or $EntryReports.Count -ne 6) {
        throw "Retained verified payload must contain exactly six reported files; found $($files.Count)."
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
                    -PermitUnsigned $PermitUnsignedApplicationArtifacts
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

if (-not (Test-IsWindows)) {
    throw 'check-bundle.ps1 must run on Windows because it validates Windows PE and Authenticode artifacts.'
}

$mainExecutable = Get-CanonicalExistingPath -LiteralPath $MainExecutablePath -RequireDirectory $false -Description 'main executable'
$nsisBundle = Get-CanonicalExistingPath -LiteralPath $NsisBundlePath -RequireDirectory $false -Description 'NSIS installer'
$nsisScript = Get-CanonicalExistingPath -LiteralPath $NsisScriptPath -RequireDirectory $false -Description 'generated installer.nsi'
$resourceRoot = Get-CanonicalExistingPath -LiteralPath $ResourceDirectory -RequireDirectory $true -Description 'resource directory'
$manifestPath = Get-CanonicalExistingPath -LiteralPath $FfmpegManifestPath -RequireDirectory $false -Description 'FFmpeg manifest'
$verifiedPayloadOutput = Resolve-VerifiedPayloadOutput -ConfiguredPath $VerifiedPayloadOutputDirectory

$resourceRootItem = Get-Item -LiteralPath $resourceRoot -Force
if (($resourceRootItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw "Resource directory must not be a reparse point: '$resourceRoot'."
}
foreach ($inputFile in @(
        [ordered]@{ path = $mainExecutable; description = 'main executable' },
        [ordered]@{ path = $nsisBundle; description = 'NSIS installer' },
        [ordered]@{ path = $nsisScript; description = 'generated installer.nsi' },
        [ordered]@{ path = $manifestPath; description = 'FFmpeg manifest' }
    )) {
    $inputItem = Get-Item -LiteralPath $inputFile.path -Force
    if (($inputItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$($inputFile.description) must not be a reparse point: '$($inputFile.path)'."
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

if (-not $AllowUnsignedInternalRc) {
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

$licenseReports = [System.Collections.Generic.List[object]]::new()
foreach ($declaration in $licenseDeclarations.GetEnumerator()) {
    $licensePath = Get-ResourceFile -Root $resourceRoot -RelativePath $declaration.Key -Description "bundled FFmpeg license file '$($declaration.Key)'"
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
foreach ($relativePath in @(
        'licenses\ffmpeg\COPYING.LGPLv3.txt',
        'licenses\ffmpeg\COPYING.GPLv3.txt',
        'licenses\ffmpeg\BUILD-INFO.json',
        'licenses\ffmpeg\SOURCE-OFFER.md'
    )) {
    $payloadSource = Get-ResourceFile -Root $resourceRoot -RelativePath $relativePath -Description "required NSIS compliance payload '$relativePath'"
    $payloadItem = Get-Item -LiteralPath $payloadSource -Force
    $expectedPayload.Add([ordered]@{
            destination = $relativePath
            sourcePath = $payloadSource
            sizeBytes = $payloadItem.Length
            sha256 = Get-Sha256 -LiteralPath $payloadSource
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
$permitUnsigned = [bool] $AllowUnsignedInternalRc
$nsisArchivePayload = Assert-NsisArchivePayload `
    -ExtractorPath $nsisExtractor `
    -InstallerPath $nsisBundle `
    -NsisHeader $nsisHeader `
    -ExpectedPayload $expectedPayload.ToArray() `
    -PermitUnsignedApplicationArtifacts $permitUnsigned `
    -VerifiedOutputConfiguration $verifiedPayloadOutput

$mainSignature = Get-SignatureReport `
    -LiteralPath $mainExecutable `
    -Description 'external UNK staging main executable' `
    -PermitUnsigned $true `
    -UnsignedAcceptanceReason $(if ($AllowUnsignedInternalRc) {
        '-AllowUnsignedInternalRc was supplied'
    }
    else {
        'the public artifact signature is verified on the embedded NSS executable'
    })
if (-not $AllowUnsignedInternalRc -and $mainSignature.status -cne 'NotSigned') {
    throw "Public release external UNK staging main executable must be NotSigned; got '$($mainSignature.status)'."
}
$nsisSignature = Get-SignatureReport -LiteralPath $nsisBundle -Description 'NSIS installer' -PermitUnsigned $permitUnsigned
$ffmpegSignature = Get-SignatureReport -LiteralPath $ffmpegPath -Description 'bundled FFmpeg executable' -PermitUnsigned $true -HashPinnedOnly

$report = [ordered]@{
    status = 'passed'
    releaseMode = if ($AllowUnsignedInternalRc) { 'internal-rc' } else { 'public-redistribution' }
    checkedAtUtc = [DateTime]::UtcNow.ToString('o')
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
}

$report | ConvertTo-Json -Depth 12
