#requires -Version 7.2

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateNotNullOrEmpty()]
    [string] $FfmpegPath,

    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string] $CandidateManifestPath = (Join-Path $PSScriptRoot '..\..\third_party\ffmpeg\minimal-windows-x64-candidate.json'),

    [Parameter(Mandatory)]
    [ValidatePattern('^[0-9a-f]{64}$')]
    [string] $ExpectedSha256,

    [Parameter()]
    [string] $OutputPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

if (-not $IsWindows -or
    [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne [System.Runtime.InteropServices.Architecture]::X64) {
    throw 'Minimal FFmpeg candidate verification requires native Windows x64.'
}

function Assert-Condition {
    param([Parameter(Mandatory)] [bool] $Condition, [Parameter(Mandatory)] [string] $Message)
    if (-not $Condition) { throw $Message }
}

function Get-CanonicalFile {
    param([Parameter(Mandatory)] [string] $Path, [Parameter(Mandatory)] [string] $Description)
    $resolved = @(Resolve-Path -LiteralPath $Path -ErrorAction Stop)
    Assert-Condition -Condition ($resolved.Count -eq 1) -Message "$Description must resolve once."
    $item = Get-Item -LiteralPath $resolved[0].ProviderPath -Force
    Assert-Condition -Condition (-not $item.PSIsContainer -and $item.Length -gt 0) -Message "$Description must be a non-empty file."
    Assert-Condition -Condition (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -eq 0) -Message "$Description must not be a reparse point."
    return [System.IO.Path]::GetFullPath($item.FullName)
}

function Assert-X64Pe {
    param([Parameter(Mandatory)] [string] $Path)
    $stream = [System.IO.File]::OpenRead($Path)
    try {
        $reader = [System.IO.BinaryReader]::new($stream)
        try {
            Assert-Condition -Condition ($reader.ReadUInt16() -eq 0x5A4D) -Message 'Candidate is missing the MZ signature.'
            $stream.Position = 0x3C
            $peOffset = $reader.ReadUInt32()
            Assert-Condition -Condition ($peOffset -le ($stream.Length - 6)) -Message 'Candidate PE offset is invalid.'
            $stream.Position = $peOffset
            Assert-Condition -Condition ($reader.ReadUInt32() -eq 0x00004550) -Message 'Candidate is missing the PE signature.'
            Assert-Condition -Condition ($reader.ReadUInt16() -eq 0x8664) -Message 'Candidate is not an x64 PE executable.'
        }
        finally { $reader.Dispose() }
    }
    finally { $stream.Dispose() }
}

function Get-PeImportedDlls {
    param([Parameter(Mandatory)] [string] $Path)

    $stream = [System.IO.File]::OpenRead($Path)
    try {
        $reader = [System.IO.BinaryReader]::new($stream)
        try {
            Assert-Condition -Condition ($reader.ReadUInt16() -eq 0x5A4D) -Message 'Candidate is missing the MZ signature.'
            $stream.Position = 0x3C
            $peOffset = [long] $reader.ReadUInt32()
            Assert-Condition -Condition ($peOffset -ge 0x40 -and $peOffset -le ($stream.Length - 24)) -Message 'Candidate PE offset is invalid.'
            $stream.Position = $peOffset
            Assert-Condition -Condition ($reader.ReadUInt32() -eq 0x00004550) -Message 'Candidate is missing the PE signature.'
            Assert-Condition -Condition ($reader.ReadUInt16() -eq 0x8664) -Message 'Candidate is not an x64 PE executable.'
            $sectionCount = [int] $reader.ReadUInt16()
            Assert-Condition -Condition ($sectionCount -gt 0 -and $sectionCount -le 96) -Message 'Candidate PE section count is invalid.'
            $stream.Position = $peOffset + 20
            $optionalHeaderSize = [int] $reader.ReadUInt16()
            $optionalHeader = $peOffset + 24
            Assert-Condition -Condition ($optionalHeaderSize -ge 224 -and ($optionalHeader + $optionalHeaderSize) -le $stream.Length) -Message 'Candidate PE optional header is invalid.'
            $stream.Position = $optionalHeader
            Assert-Condition -Condition ($reader.ReadUInt16() -eq 0x20B) -Message 'Candidate is not a PE32+ executable.'
            $stream.Position = $optionalHeader + 60
            $sizeOfHeaders = [uint32] $reader.ReadUInt32()
            $stream.Position = $optionalHeader + 108
            $directoryCount = [uint32] $reader.ReadUInt32()
            Assert-Condition -Condition ($directoryCount -ge 14) -Message 'Candidate PE data-directory table is incomplete.'
            $stream.Position = $optionalHeader + 120
            $importRva = [uint32] $reader.ReadUInt32()
            $importSize = [uint32] $reader.ReadUInt32()
            $stream.Position = $optionalHeader + 216
            $delayImportRva = [uint32] $reader.ReadUInt32()
            $delayImportSize = [uint32] $reader.ReadUInt32()
            Assert-Condition -Condition ($delayImportRva -eq 0 -and $delayImportSize -eq 0) -Message 'Candidate uses a delay-import table that is outside the minimal dependency contract.'
            Assert-Condition -Condition ($importRva -ne 0 -and $importSize -ge 20) -Message 'Candidate PE import table is missing.'

            $sections = [System.Collections.Generic.List[object]]::new()
            $stream.Position = $optionalHeader + $optionalHeaderSize
            for ($index = 0; $index -lt $sectionCount; $index += 1) {
                $nameBytes = $reader.ReadBytes(8)
                Assert-Condition -Condition ($nameBytes.Length -eq 8) -Message 'Candidate PE section table is truncated.'
                $virtualSize = [uint32] $reader.ReadUInt32()
                $virtualAddress = [uint32] $reader.ReadUInt32()
                $rawSize = [uint32] $reader.ReadUInt32()
                $rawPointer = [uint32] $reader.ReadUInt32()
                $stream.Position += 12
                [void] $reader.ReadUInt32()
                $sections.Add([ordered]@{
                        virtualSize = $virtualSize
                        virtualAddress = $virtualAddress
                        rawSize = $rawSize
                        rawPointer = $rawPointer
                    })
            }

            $rvaToOffset = {
                param([uint32] $Rva)
                if ($Rva -lt $sizeOfHeaders) {
                    Assert-Condition -Condition ([long] $Rva -lt $stream.Length) -Message 'Candidate PE header RVA exceeds the file.'
                    return [long] $Rva
                }
                foreach ($section in $sections) {
                    $span = [Math]::Max([long] $section.virtualSize, [long] $section.rawSize)
                    $start = [long] $section.virtualAddress
                    if ([long] $Rva -ge $start -and [long] $Rva -lt ($start + $span)) {
                        $delta = [long] $Rva - $start
                        Assert-Condition -Condition ($delta -lt [long] $section.rawSize) -Message 'Candidate PE RVA points into an unbacked virtual section tail.'
                        $offset = [long] $section.rawPointer + $delta
                        Assert-Condition -Condition ($offset -ge 0 -and $offset -lt $stream.Length) -Message 'Candidate PE RVA maps outside the file.'
                        return $offset
                    }
                }
                throw ('Candidate PE RVA 0x{0:x8} is not mapped by a section.' -f $Rva)
            }
            $readAsciiName = {
                param([uint32] $Rva)
                $offset = [long] (& $rvaToOffset $Rva)
                $stream.Position = $offset
                $bytes = [System.Collections.Generic.List[byte]]::new()
                for ($index = 0; $index -lt 260; $index += 1) {
                    Assert-Condition -Condition ($stream.Position -lt $stream.Length) -Message 'Candidate PE import name is truncated.'
                    $value = $reader.ReadByte()
                    if ($value -eq 0) { break }
                    $bytes.Add($value)
                }
                Assert-Condition -Condition ($bytes.Count -gt 0 -and $bytes.Count -lt 260) -Message 'Candidate PE import name is invalid.'
                $name = [System.Text.Encoding]::ASCII.GetString($bytes.ToArray())
                Assert-Condition -Condition ($name -match '^[A-Za-z0-9._-]+\.dll$') -Message "Candidate PE import name is unsafe: '$name'."
                return $name
            }

            $importOffset = [long] (& $rvaToOffset $importRva)
            $imports = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
            $terminated = $false
            $descriptorCountLimit = [Math]::Min(4096, [int] [Math]::Floor([double] $importSize / 20))
            Assert-Condition -Condition ($descriptorCountLimit -gt 0) -Message 'Candidate PE import directory is too small.'
            for ($index = 0; $index -lt $descriptorCountLimit; $index += 1) {
                $descriptorOffset = $importOffset + ($index * 20)
                Assert-Condition -Condition (($descriptorOffset + 20) -le $stream.Length) -Message 'Candidate PE import descriptor table is truncated.'
                $stream.Position = $descriptorOffset
                $originalFirstThunk = $reader.ReadUInt32()
                $timeDateStamp = $reader.ReadUInt32()
                $forwarderChain = $reader.ReadUInt32()
                $nameRva = $reader.ReadUInt32()
                $firstThunk = $reader.ReadUInt32()
                if (($originalFirstThunk -bor $timeDateStamp -bor $forwarderChain -bor $nameRva -bor $firstThunk) -eq 0) {
                    $terminated = $true
                    break
                }
                Assert-Condition -Condition ($nameRva -ne 0) -Message 'Candidate PE import descriptor has no DLL name.'
                [void] $imports.Add((& $readAsciiName ([uint32] $nameRva)))
            }
            Assert-Condition -Condition $terminated -Message 'Candidate PE import descriptor table is not terminated.'
            Assert-Condition -Condition ($imports.Count -gt 0) -Message 'Candidate PE import set is empty.'
            return @($imports | Sort-Object)
        }
        finally { $reader.Dispose() }
    }
    finally { $stream.Dispose() }
}

function Get-CanonicalDirectory {
    param([Parameter(Mandatory)] [string] $Path, [Parameter(Mandatory)] [string] $Description)
    $resolved = @(Resolve-Path -LiteralPath $Path -ErrorAction Stop)
    Assert-Condition -Condition ($resolved.Count -eq 1) -Message "$Description must resolve once."
    $item = Get-Item -LiteralPath $resolved[0].ProviderPath -Force
    Assert-Condition -Condition ($item.PSIsContainer) -Message "$Description must be a directory."
    Assert-Condition -Condition (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -eq 0) -Message "$Description must not be a reparse point."
    return [System.IO.Path]::GetFullPath($item.FullName)
}

function Get-ArtifactRootFile {
    param(
        [Parameter(Mandatory)] [string] $Root,
        [Parameter(Mandatory)] [string] $FileName,
        [Parameter(Mandatory)] [string] $Description
    )
    Assert-Condition -Condition ([System.IO.Path]::GetFileName($FileName) -ceq $FileName -and $FileName.IndexOf([char] 0) -lt 0) -Message "$Description file name is unsafe."
    $candidate = [System.IO.Path]::GetFullPath((Join-Path $Root $FileName))
    $prefix = $Root.TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
    Assert-Condition -Condition ($candidate.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) -Message "$Description escapes the artifact root."
    return Get-CanonicalFile -Path $candidate -Description $Description
}

function Invoke-CheckedProcess {
    param(
        [Parameter(Mandatory)] [string] $Executable,
        [Parameter(Mandatory)] [string[]] $Arguments,
        [Parameter(Mandatory)] [int] $TimeoutSeconds,
        [Parameter(Mandatory)] [string] $Description
    )
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Executable
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = [System.Text.Encoding]::UTF8
    $startInfo.StandardErrorEncoding = [System.Text.Encoding]::UTF8
    [void] $startInfo.Environment.Remove('FFREPORT')
    $startInfo.Environment['AV_LOG_FORCE_NOCOLOR'] = '1'
    foreach ($argument in $Arguments) { [void] $startInfo.ArgumentList.Add($argument) }
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        Assert-Condition -Condition $process.Start() -Message "Could not start $Description."
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            try { $process.Kill($true) } catch { Write-Verbose $_ }
            $process.WaitForExit()
            throw "$Description timed out after $TimeoutSeconds seconds."
        }
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        Assert-Condition -Condition ($process.ExitCode -eq 0) -Message "$Description failed with exit code $($process.ExitCode).`n$stdout`n$stderr"
        return [ordered]@{ stdout = $stdout; stderr = $stderr; combined = "$stdout`n$stderr" }
    }
    finally { $process.Dispose() }
}

function Assert-Capability {
    param(
        [Parameter(Mandatory)] [string] $Kind,
        [Parameter(Mandatory)] [string] $Name,
        [Parameter(Mandatory)] [string] $Text
    )
    $escaped = [regex]::Escape($Name)
    $pattern = switch ($Kind) {
        'protocols' { "(?m)^\s*$escaped\s*$" }
        'demuxers' { "(?m)^\s*D\s+(?:[A-Za-z0-9_]+,)*$escaped(?:,|\s)" }
        'decoders' { "(?m)^\s*[A-Z\.]{6}\s+$escaped\s" }
        'filters' { "(?m)^\s*[A-Z\.]+\s+$escaped\s" }
        'encoders' { "(?m)^\s*[A-Z\.]{6}\s+$escaped\s" }
        'muxers' { "(?m)^\s*E\s+$escaped\s" }
        default { throw "Unsupported capability kind '$Kind'." }
    }
    Assert-Condition -Condition ([regex]::IsMatch($Text, $pattern)) -Message "Candidate is missing required $Kind capability '$Name'."
}

$ffmpeg = Get-CanonicalFile -Path $FfmpegPath -Description 'minimal FFmpeg candidate'
$manifestPath = Get-CanonicalFile -Path $CandidateManifestPath -Description 'minimal FFmpeg candidate manifest'
$manifest = Get-Content -Raw -LiteralPath $manifestPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
Assert-Condition -Condition ([long] $manifest.schemaVersion -eq 1) -Message 'Unsupported candidate manifest schemaVersion.'
Assert-Condition -Condition ([string] $manifest.status -ceq 'candidate-not-promoted') -Message 'Candidate manifest must remain explicitly unpromoted.'
Assert-X64Pe -Path $ffmpeg

$item = Get-Item -LiteralPath $ffmpeg -Force
$actualHash = (Get-FileHash -LiteralPath $ffmpeg -Algorithm SHA256).Hash.ToLowerInvariant()
Assert-Condition -Condition ($actualHash -ceq $ExpectedSha256.ToLowerInvariant()) -Message 'Candidate SHA-256 does not match the build evidence.'

$artifactRoot = Get-CanonicalDirectory -Path ([System.IO.Directory]::GetParent($ffmpeg).FullName) -Description 'candidate artifact root'
$evidenceFileNames = @(
    'BUILD-METADATA.json',
    'compiler-version.txt',
    'config.h',
    'config.mak',
    'configure-flags.txt',
    'ffmpeg-corresponding-source.tar.xz',
    'ffmpeg.exe',
    'pe-imports.txt'
)
$checksumPath = Get-ArtifactRootFile -Root $artifactRoot -FileName 'SHA256SUMS.txt' -Description 'candidate checksum manifest'
$checksumLines = @(Get-Content -LiteralPath $checksumPath -Encoding UTF8 | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
$checksumEntries = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
$evidenceHashes = [ordered]@{}
foreach ($line in $checksumLines) {
    Assert-Condition -Condition ($line -cmatch '^([0-9a-f]{64})  ([A-Za-z0-9._-]+)$') -Message "Candidate checksum manifest contains an invalid line: '$line'."
    $expectedEvidenceHash = [string] $Matches[1]
    $evidenceFileName = [string] $Matches[2]
    Assert-Condition -Condition ($evidenceFileName -cin $evidenceFileNames -and $checksumEntries.Add($evidenceFileName)) -Message "Candidate checksum manifest contains an unknown or duplicate file '$evidenceFileName'."
    $evidencePath = Get-ArtifactRootFile -Root $artifactRoot -FileName $evidenceFileName -Description "candidate evidence '$evidenceFileName'"
    $evidenceHash = (Get-FileHash -LiteralPath $evidencePath -Algorithm SHA256).Hash.ToLowerInvariant()
    Assert-Condition -Condition ($evidenceHash -ceq $expectedEvidenceHash) -Message "Candidate evidence '$evidenceFileName' does not match SHA256SUMS.txt."
    $evidenceHashes[$evidenceFileName] = $evidenceHash
}
Assert-Condition -Condition ($checksumEntries.Count -eq $evidenceFileNames.Count -and @($evidenceFileNames | Where-Object { -not $checksumEntries.Contains($_) }).Count -eq 0) -Message 'Candidate checksum manifest does not cover the exact required evidence set.'
$artifactItems = @(Get-ChildItem -LiteralPath $artifactRoot -Force)
foreach ($artifactItem in $artifactItems) {
    Assert-Condition -Condition (-not $artifactItem.PSIsContainer -and ($artifactItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -eq 0) -Message "Candidate artifact root contains a directory or reparse point: '$($artifactItem.Name)'."
}
$allowedArtifactNames = @($evidenceFileNames + 'SHA256SUMS.txt')
Assert-Condition -Condition ($artifactItems.Count -eq $allowedArtifactNames.Count -and @($artifactItems | Where-Object { $_.Name -cnotin $allowedArtifactNames }).Count -eq 0) -Message 'Candidate artifact root contains an undeclared file or is missing required evidence.'

$metadataPath = Get-ArtifactRootFile -Root $artifactRoot -FileName 'BUILD-METADATA.json' -Description 'candidate build metadata'
$metadata = Get-Content -Raw -LiteralPath $metadataPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
Assert-Condition -Condition ([long] $metadata.schemaVersion -eq 1 -and [string] $metadata.status -ceq 'built-candidate-not-promoted') -Message 'Candidate build metadata schema or status is invalid.'
Assert-Condition -Condition ([string] $metadata.sourceCommit -ceq [string] $manifest.source.commit) -Message 'Candidate build metadata source commit does not match the manifest.'
Assert-Condition -Condition ([string] $metadata.targetTriple -ceq [string] $manifest.build.targetTriple) -Message 'Candidate build metadata target triple does not match the manifest.'
Assert-Condition -Condition ([string] $metadata.licenseExpression -ceq [string] $manifest.build.licenseExpression) -Message 'Candidate build metadata license expression does not match the manifest.'
$manifestFlags = @($manifest.build.configureFlags)
$metadataFlags = @($metadata.configureFlags)
Assert-Condition -Condition ($metadataFlags.Count -eq $manifestFlags.Count) -Message 'Candidate build metadata configure flag count does not match the manifest.'
for ($index = 0; $index -lt $manifestFlags.Count; $index += 1) {
    Assert-Condition -Condition ([string] $metadataFlags[$index] -ceq [string] $manifestFlags[$index]) -Message "Candidate build metadata configure flag $index does not match the manifest."
}
Assert-Condition -Condition (@($manifest.build.externalLibraries).Count -eq 0 -and @($metadata.externalLibraries).Count -eq 0) -Message 'Minimal candidate must not declare external libraries.'
Assert-Condition -Condition ([string] $metadata.executable.fileName -ceq 'ffmpeg.exe' -and [long] $metadata.executable.sizeBytes -eq $item.Length -and [string] $metadata.executable.sha256 -ceq $actualHash) -Message 'Candidate build metadata executable record does not match ffmpeg.exe.'

$peImports = @(Get-PeImportedDlls -Path $ffmpeg)
$allowedImports = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
foreach ($allowedImport in @($manifest.runtimeContract.allowedSystemDllImports)) {
    Assert-Condition -Condition ($allowedImport -is [string] -and [string] $allowedImport -cmatch '^[A-Za-z0-9._-]+\.dll$' -and $allowedImports.Add([string] $allowedImport)) -Message 'Candidate manifest contains an unsafe or duplicate allowed system DLL import.'
}
$unexpectedImports = @($peImports | Where-Object { -not $allowedImports.Contains($_) })
Assert-Condition -Condition ($allowedImports.Count -gt 0 -and $unexpectedImports.Count -eq 0) -Message "Candidate imports DLLs outside the Windows system allowlist: $($unexpectedImports -join ', ')."
$objdumpPath = Get-ArtifactRootFile -Root $artifactRoot -FileName 'pe-imports.txt' -Description 'candidate objdump import evidence'
$objdumpImports = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
foreach ($line in Get-Content -LiteralPath $objdumpPath -Encoding UTF8) {
    if ($line -match '^\s*DLL Name:\s*([^\s]+)\s*$') {
        [void] $objdumpImports.Add([string] $Matches[1])
    }
}
Assert-Condition -Condition ($objdumpImports.Count -eq $peImports.Count -and @($peImports | Where-Object { -not $objdumpImports.Contains($_) }).Count -eq 0) -Message 'Cross-toolchain objdump imports do not match the Windows PE parser.'

$artifactEvidence = [ordered]@{
    root = $artifactRoot
    checksumManifestSha256 = (Get-FileHash -LiteralPath $checksumPath -Algorithm SHA256).Hash.ToLowerInvariant()
    files = $evidenceHashes
    sourceArchiveSha256 = [string] $evidenceHashes['ffmpeg-corresponding-source.tar.xz']
    peImports = $peImports
}

$version = Invoke-CheckedProcess -Executable $ffmpeg -Arguments @('-hide_banner', '-version') -TimeoutSeconds 10 -Description 'FFmpeg version inspection'
$commitShort = ([string] $manifest.source.commit).Substring(0, 12)
Assert-Condition -Condition ($version.combined.Contains($commitShort, [System.StringComparison]::OrdinalIgnoreCase)) -Message 'Candidate version does not identify the pinned FFmpeg commit.'
$license = Invoke-CheckedProcess -Executable $ffmpeg -Arguments @('-hide_banner', '-L') -TimeoutSeconds 10 -Description 'FFmpeg license inspection'
Assert-Condition -Condition ($license.combined -match 'GNU Lesser General Public License') -Message 'Candidate does not report an LGPL license.'

$buildConf = Invoke-CheckedProcess -Executable $ffmpeg -Arguments @('-hide_banner', '-buildconf') -TimeoutSeconds 10 -Description 'FFmpeg build configuration inspection'
foreach ($flag in @($manifest.build.configureFlags)) {
    Assert-Condition -Condition ($buildConf.combined.Contains([string] $flag, [System.StringComparison]::Ordinal)) -Message "Candidate build configuration is missing '$flag'."
}
foreach ($flag in @($manifest.build.forbiddenFlags)) {
    Assert-Condition -Condition (-not $buildConf.combined.Contains([string] $flag, [System.StringComparison]::Ordinal)) -Message "Candidate build configuration contains forbidden '$flag'."
}
Assert-Condition -Condition ($buildConf.combined -notmatch '(?m)--enable-lib[^\s]+') -Message 'Candidate unexpectedly enables an external library.'

$capabilityCommands = [ordered]@{
    protocols = @('-hide_banner', '-protocols')
    demuxers = @('-hide_banner', '-demuxers')
    decoders = @('-hide_banner', '-decoders')
    filters = @('-hide_banner', '-filters')
    encoders = @('-hide_banner', '-encoders')
    muxers = @('-hide_banner', '-muxers')
}
$capabilityReports = [ordered]@{}
foreach ($kind in $capabilityCommands.Keys) {
    $result = Invoke-CheckedProcess -Executable $ffmpeg -Arguments $capabilityCommands[$kind] -TimeoutSeconds 10 -Description "FFmpeg $kind inspection"
    foreach ($name in @($manifest.runtimeContract.requiredCapabilities.$kind)) {
        Assert-Capability -Kind $kind -Name ([string] $name) -Text $result.combined
    }
    $capabilityReports[$kind] = @($manifest.runtimeContract.requiredCapabilities.$kind)
}

$tempParent = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd('\')
$smokeRoot = Join-Path $tempParent ('vhm-minimal-ffmpeg-' + [Guid]::NewGuid().ToString('N'))
[void] [System.IO.Directory]::CreateDirectory($smokeRoot)
try {
    $smokeFixtures = @($manifest.runtimeContract.smokeFixture)
    if ($manifest.runtimeContract.PSObject.Properties.Name -contains 'additionalSmokeFixtures') {
        $smokeFixtures += @($manifest.runtimeContract.additionalSmokeFixtures)
    }
    Assert-Condition -Condition ($smokeFixtures.Count -ge 1) -Message 'At least one smoke fixture is required.'
    $smokeReports = @()
    for ($fixtureIndex = 0; $fixtureIndex -lt $smokeFixtures.Count; $fixtureIndex++) {
        $fixture = $smokeFixtures[$fixtureIndex]
        $codecProperty = $fixture.PSObject.Properties['codec']
        $codec = if ($null -eq $codecProperty -or [string]::IsNullOrWhiteSpace([string] $codecProperty.Value)) { "fixture-$fixtureIndex" } else { [string] $codecProperty.Value }
        Assert-Condition -Condition ([string] $fixture.encoding -ceq 'base64') -Message "Unsupported $codec smoke fixture encoding."
        $fixtureInputPath = Join-Path $smokeRoot ("fixture-$fixtureIndex.mp4")
        $thumbnailResultPath = Join-Path $smokeRoot ("thumbnail-$fixtureIndex.jpg")
        $fixtureBytes = [Convert]::FromBase64String([string] $fixture.base64)
        Assert-Condition -Condition ($fixtureBytes.Length -eq [long] $fixture.sizeBytes) -Message "$codec smoke fixture size mismatch."
        [System.IO.File]::WriteAllBytes($fixtureInputPath, $fixtureBytes)
        $fixtureHash = (Get-FileHash -LiteralPath $fixtureInputPath -Algorithm SHA256).Hash.ToLowerInvariant()
        Assert-Condition -Condition ($fixtureHash -ceq [string] $fixture.sha256) -Message "$codec smoke fixture SHA-256 mismatch."
        $arguments = @($manifest.runtimeContract.thumbnailArguments | ForEach-Object {
                if ([string] $_ -ceq '{input}') { $fixtureInputPath }
                elseif ([string] $_ -ceq '{output}') { $thumbnailResultPath }
                else { [string] $_ }
            })
        [void] (Invoke-CheckedProcess -Executable $ffmpeg -Arguments $arguments -TimeoutSeconds ([int] $manifest.runtimeContract.processTimeoutSeconds) -Description "minimal FFmpeg $codec thumbnail smoke")
        $thumbnailResult = Get-Item -LiteralPath $thumbnailResultPath -Force -ErrorAction Stop
        Assert-Condition -Condition ($thumbnailResult.Length -gt 4 -and $thumbnailResult.Length -le [long] $manifest.runtimeContract.maximumOutputBytes) -Message "$codec thumbnail output size is invalid."
        $thumbnailBytes = [System.IO.File]::ReadAllBytes($thumbnailResultPath)
        Assert-Condition -Condition ($thumbnailBytes[0] -eq 0xFF -and $thumbnailBytes[1] -eq 0xD8 -and $thumbnailBytes[-2] -eq 0xFF -and $thumbnailBytes[-1] -eq 0xD9) -Message "$codec thumbnail output is not a complete JPEG."
        $smokeReports += [ordered]@{
            codec = $codec
            fixtureSha256 = $fixtureHash
            outputSizeBytes = $thumbnailResult.Length
            outputSha256 = (Get-FileHash -LiteralPath $thumbnailResultPath -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
    $smoke = $smokeReports[0]
}
finally {
    $smokeFull = [System.IO.Path]::GetFullPath($smokeRoot)
    $safeName = [System.IO.Path]::GetFileName($smokeFull) -cmatch '^vhm-minimal-ffmpeg-[0-9a-f]{32}$'
    $safeParent = [string]::Equals([System.IO.Directory]::GetParent($smokeFull).FullName.TrimEnd('\'), $tempParent, [System.StringComparison]::OrdinalIgnoreCase)
    if ($safeName -and $safeParent -and [System.IO.Directory]::Exists($smokeFull)) {
        Remove-Item -LiteralPath $smokeFull -Recurse -Force
    }
}

$report = [ordered]@{
    schemaVersion = 1
    status = 'passed-candidate-not-promoted'
    checkedAtUtc = [DateTime]::UtcNow.ToString('o')
    executable = [ordered]@{ sizeBytes = $item.Length; sha256 = $actualHash }
    sourceCommit = [string] $manifest.source.commit
    licenseExpression = [string] $manifest.build.licenseExpression
    externalLibraries = @()
    artifactEvidence = $artifactEvidence
    requiredCapabilities = $capabilityReports
    smoke = $smoke
    smokeFixtures = @($smokeReports)
    promotionAuthorized = $false
}
$json = $report | ConvertTo-Json -Depth 10
if (-not [string]::IsNullOrWhiteSpace($OutputPath)) {
    $outputFull = [System.IO.Path]::GetFullPath($OutputPath)
    Assert-Condition -Condition (-not [System.IO.File]::Exists($outputFull)) -Message 'Candidate verification output already exists.'
    Assert-Condition -Condition ([System.IO.Directory]::Exists([System.IO.Directory]::GetParent($outputFull).FullName)) -Message 'Candidate verification output parent is missing.'
    $json | Set-Content -LiteralPath $outputFull -Encoding UTF8
}
$json
