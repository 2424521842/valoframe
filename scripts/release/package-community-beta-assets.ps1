#requires -Version 7.2

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidatePattern('^v[0-9]+\.[0-9]+\.[0-9]+-beta\.[1-9][0-9]*$')]
    [string] $ReleaseTag,

    [Parameter(Mandatory)]
    [ValidatePattern('^[0-9a-f]{40}$')]
    [string] $SourceCommit,

    [Parameter(Mandatory)]
    [ValidatePattern('^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$')]
    [string] $RepositorySlug,

    [string] $MirrorUrl = '',
    [string] $MirrorPassword = '',
    [string] $MirrorFileName = '',
    [string] $MirrorSha256 = '',

    [Parameter(Mandatory)] [ValidateNotNullOrEmpty()] [string] $InstallerPath,
    [Parameter(Mandatory)] [ValidateNotNullOrEmpty()] [string] $FfmpegPackageRoot,
    [Parameter(Mandatory)] [ValidateNotNullOrEmpty()] [string] $FfmpegArchivePath,
    [Parameter(Mandatory)] [ValidateNotNullOrEmpty()] [string] $FfmpegManifestPath,
    [Parameter(Mandatory)] [ValidateNotNullOrEmpty()] [string] $BundleReportPath,
    [Parameter(Mandatory)] [ValidateNotNullOrEmpty()] [string] $PublicPreflightReportPath,
    [Parameter(Mandatory)] [ValidateNotNullOrEmpty()] [string] $StrictBundleBlockReportPath,
    [Parameter(Mandatory)] [ValidateNotNullOrEmpty()] [string] $SmokeReportPath,
    [Parameter(Mandatory)] [ValidateNotNullOrEmpty()] [string] $ResourceLicenseRoot,
    [Parameter(Mandatory)] [ValidateNotNullOrEmpty()] [string] $OutputDirectory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-Condition {
    param([Parameter(Mandatory)] [bool] $Condition, [Parameter(Mandatory)] [string] $Message)
    if (-not $Condition) { throw $Message }
}

function Get-RegularFile {
    param([Parameter(Mandatory)] [string] $Path, [Parameter(Mandatory)] [string] $Description)
    $resolved = @(Resolve-Path -LiteralPath $Path -ErrorAction Stop)
    Assert-Condition -Condition ($resolved.Count -eq 1) -Message "$Description must resolve exactly once."
    $item = Get-Item -LiteralPath $resolved[0].ProviderPath -Force
    Assert-Condition -Condition (-not $item.PSIsContainer -and $item.Length -gt 0 -and ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -eq 0) -Message "$Description must be a non-empty regular file without reparse points."
    return [System.IO.Path]::GetFullPath($item.FullName)
}

function Get-RegularDirectory {
    param([Parameter(Mandatory)] [string] $Path, [Parameter(Mandatory)] [string] $Description)
    $resolved = @(Resolve-Path -LiteralPath $Path -ErrorAction Stop)
    Assert-Condition -Condition ($resolved.Count -eq 1) -Message "$Description must resolve exactly once."
    $item = Get-Item -LiteralPath $resolved[0].ProviderPath -Force
    Assert-Condition -Condition ($item.PSIsContainer -and ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -eq 0) -Message "$Description must be a regular directory without reparse points."
    return [System.IO.Path]::GetFullPath($item.FullName)
}

function Get-Sha256 {
    param([Parameter(Mandatory)] [string] $Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
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
    param(
        [Parameter(Mandatory)] [string] $Root,
        [Parameter(Mandatory)] [string] $RelativePath,
        [Parameter(Mandatory)] [string] $Description
    )
    Assert-Condition -Condition (-not [System.IO.Path]::IsPathRooted($RelativePath) -and $RelativePath.IndexOf([char] 0) -lt 0) -Message "$Description path is unsafe."
    $segments = $RelativePath.Replace('\', '/').Split('/')
    Assert-Condition -Condition (@($segments | Where-Object { [string]::IsNullOrWhiteSpace($_) -or $_ -in @('.', '..') }).Count -eq 0) -Message "$Description path contains an unsafe segment."
    $candidate = [System.IO.Path]::GetFullPath((Join-Path $Root ($segments -join [System.IO.Path]::DirectorySeparatorChar)))
    Assert-Condition -Condition (Test-PathWithinOrEqualRoot -Root $Root -Path $candidate) -Message "$Description escapes the technical package root."
    Assert-Condition -Condition (-not [string]::Equals([System.IO.Path]::GetFullPath($Root).TrimEnd('\', '/'), $candidate.TrimEnd('\', '/'), [System.StringComparison]::OrdinalIgnoreCase)) -Message "$Description must name a file below the technical package root."
    return Get-RegularFile -Path $candidate -Description $Description
}

function Write-LfText {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [string] $Text,
        [Parameter(Mandatory)] [System.Text.Encoding] $Encoding
    )
    $normalized = $Text.Replace("`r`n", "`n").Replace("`r", "`n")
    if (-not $normalized.EndsWith("`n", [System.StringComparison]::Ordinal)) {
        $normalized += "`n"
    }
    [System.IO.File]::WriteAllText($Path, $normalized, $Encoding)
}

function Add-ZipEntry {
    param(
        [Parameter(Mandatory)] [System.IO.Compression.ZipArchive] $Archive,
        [Parameter(Mandatory)] [string] $EntryPath,
        [Parameter(Mandatory)] [string] $SourcePath
    )
    Assert-Condition -Condition ($EntryPath -cmatch '^[A-Za-z0-9][A-Za-z0-9._/-]*$' -and -not $EntryPath.Contains('..')) -Message "Unsafe ZIP entry '$EntryPath'."
    $entry = $Archive.CreateEntry($EntryPath, [System.IO.Compression.CompressionLevel]::Optimal)
    $entry.LastWriteTime = [DateTimeOffset]::new(2026, 1, 1, 0, 0, 0, [TimeSpan]::Zero)
    $input = [System.IO.File]::OpenRead($SourcePath)
    $output = $entry.Open()
    try { $input.CopyTo($output) }
    finally {
        $output.Dispose()
        $input.Dispose()
    }
}

$mirrorValues = @($MirrorUrl, $MirrorPassword, $MirrorFileName, $MirrorSha256)
$hasMirror = @($mirrorValues | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }).Count -gt 0
Assert-Condition -Condition (-not $hasMirror -or @($mirrorValues | Where-Object { [string]::IsNullOrWhiteSpace($_) }).Count -eq 0) -Message 'Mirror metadata must be entirely empty or fully specified.'
if ($hasMirror) {
    $mirrorUri = $null
    Assert-Condition -Condition (
        [Uri]::TryCreate($MirrorUrl, [UriKind]::Absolute, [ref] $mirrorUri) -and
        $mirrorUri.Scheme -ceq 'https' -and
        [string]::IsNullOrEmpty($mirrorUri.UserInfo) -and
        $MirrorUrl -cnotmatch '[\s`<>()\[\]]'
    ) -Message 'Mirror URL must be a safe absolute HTTPS URL.'
    Assert-Condition -Condition ($MirrorPassword -cmatch '^[A-Za-z0-9]{1,32}$') -Message 'Mirror password must be 1-32 ASCII letters or digits.'
    Assert-Condition -Condition ($MirrorFileName -cmatch '^[^\x00-\x1f\\/:*?"<>|]+\.exe$' -and -not $MirrorFileName.Contains('`')) -Message 'Mirror file name must be a safe .exe file name.'
    Assert-Condition -Condition ($MirrorSha256 -cmatch '^[0-9a-f]{64}$') -Message 'Mirror SHA-256 must be 64 lowercase hexadecimal characters.'
}

$installer = Get-RegularFile -Path $InstallerPath -Description 'Community Beta installer'
$ffmpegPackage = Get-RegularDirectory -Path $FfmpegPackageRoot -Description 'Community Beta FFmpeg package root'
$ffmpegArchive = Get-RegularFile -Path $FfmpegArchivePath -Description 'minimal FFmpeg binary archive'
Assert-Condition -Condition (-not (Test-PathWithinOrEqualRoot -Root $ffmpegPackage -Path $ffmpegArchive)) -Message 'Minimal FFmpeg binary archive must remain outside the hash-bound technical package root.'
$ffmpegManifestFile = Get-RegularFile -Path $FfmpegManifestPath -Description 'Community Beta FFmpeg manifest'
$bundleReportFile = Get-RegularFile -Path $BundleReportPath -Description 'Community Beta bundle report'
$preflightReportFile = Get-RegularFile -Path $PublicPreflightReportPath -Description 'strict public preflight report'
$strictBundleBlockFile = Get-RegularFile -Path $StrictBundleBlockReportPath -Description 'strict public bundle block report'
$smokeReportFile = Get-RegularFile -Path $SmokeReportPath -Description 'Community Beta startup smoke report'
$licenseRoot = Get-RegularDirectory -Path $ResourceLicenseRoot -Description 'resource license root'

$technicalManifestPath = Get-PackageFile -Root $ffmpegPackage -RelativePath 'COMMUNITY-BETA-FFMPEG-PACKAGE.json' -Description 'technical FFmpeg package manifest'
$technicalChecksumPath = Get-PackageFile -Root $ffmpegPackage -RelativePath 'SHA256SUMS.txt' -Description 'technical FFmpeg package checksum manifest'
$technicalChecksums = [System.Collections.Generic.Dictionary[string, string]]::new([System.StringComparer]::Ordinal)
foreach ($line in Get-Content -LiteralPath $technicalChecksumPath -Encoding UTF8) {
    Assert-Condition -Condition ($line -cmatch '^([0-9a-f]{64})  ([A-Za-z0-9][A-Za-z0-9._/-]*)$') -Message 'Technical FFmpeg checksum manifest contains an invalid line.'
    $relative = [string] $Matches[2]
    Assert-Condition -Condition (-not $relative.Contains('..') -and $technicalChecksums.TryAdd($relative, [string] $Matches[1])) -Message 'Technical FFmpeg checksum manifest contains an unsafe or duplicate path.'
    $file = Get-PackageFile -Root $ffmpegPackage -RelativePath $relative -Description "technical FFmpeg package file '$relative'"
    Assert-Condition -Condition ((Get-Sha256 -Path $file) -ceq [string] $Matches[1]) -Message "Technical FFmpeg package hash mismatch for '$relative'."
}
$actualTechnicalFiles = @(Get-ChildItem -LiteralPath $ffmpegPackage -File -Recurse -Force | Where-Object { $_.FullName -cne $technicalChecksumPath })
Assert-Condition -Condition ($actualTechnicalFiles.Count -eq $technicalChecksums.Count) -Message 'Technical FFmpeg package checksum coverage is incomplete.'
foreach ($actualTechnicalFile in $actualTechnicalFiles) {
    $actualRelative = [System.IO.Path]::GetRelativePath($ffmpegPackage, $actualTechnicalFile.FullName).Replace('\', '/')
    Assert-Condition -Condition ($technicalChecksums.ContainsKey($actualRelative)) -Message "Technical FFmpeg package checksum manifest omits '$actualRelative'."
}
$technicalManifest = Get-Content -Raw -LiteralPath $technicalManifestPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
Assert-Condition -Condition (
    [long] $technicalManifest.schemaVersion -eq 1 -and
    [string] $technicalManifest.status -ceq 'prepared-community-beta-candidate-not-public-release-approved' -and
    [string] $technicalManifest.channel -ceq 'community-beta' -and
    $technicalManifest.technicalPromotion.communityBetaDistributionAuthorized -eq $false -and
    $technicalManifest.technicalPromotion.publicReleaseApproved -eq $false
) -Message 'Technical FFmpeg package manifest is not in the required unpromoted state.'

Assert-Condition -Condition (-not (Test-Path -LiteralPath $OutputDirectory)) -Message 'Community Beta release asset output must not already exist.'
$output = [System.IO.Path]::GetFullPath($OutputDirectory)
$outputParent = [System.IO.Directory]::GetParent($output).FullName
Assert-Condition -Condition (Test-Path -LiteralPath $outputParent -PathType Container) -Message 'Community Beta release asset output parent is missing.'

$manifest = Get-Content -Raw -LiteralPath $ffmpegManifestFile -Encoding UTF8 | ConvertFrom-Json -Depth 100
$bundleReport = Get-Content -Raw -LiteralPath $bundleReportFile -Encoding UTF8 | ConvertFrom-Json -Depth 100
$preflightReport = Get-Content -Raw -LiteralPath $preflightReportFile -Encoding UTF8 | ConvertFrom-Json -Depth 100
$strictBundleBlock = Get-Content -Raw -LiteralPath $strictBundleBlockFile -Encoding UTF8 | ConvertFrom-Json -Depth 100
$smokeReport = Get-Content -Raw -LiteralPath $smokeReportFile -Encoding UTF8 | ConvertFrom-Json -Depth 100

Assert-Condition -Condition ([string] $manifest.releaseChannel -ceq 'community-beta' -and $manifest.productionPromotionAuthorized -eq $false -and [string] $manifest.provider.releaseTag -ceq $ReleaseTag) -Message 'FFmpeg manifest is not bound to this Community Beta tag.'
Assert-Condition -Condition (
    [string] $manifest.build.applicationSourceCommit -ceq $SourceCommit -and
    [string] $manifest.sourceCompliance.buildScriptsSource.commit -ceq $SourceCommit
) -Message 'FFmpeg manifest is not bound to the approved application source commit.'
Assert-Condition -Condition ([string] $bundleReport.status -ceq 'passed' -and [string] $bundleReport.releaseMode -ceq 'unsigned-community-beta' -and $bundleReport.strictPublicReleaseApproved -eq $false) -Message 'Bundle report did not pass the unsigned Community Beta gate.'
Assert-Condition -Condition ([string] $preflightReport.status -ceq 'blocked' -and [long] $preflightReport.blockerCount -gt 0) -Message 'Strict public preflight must remain blocked.'
Assert-Condition -Condition (
    [string] $strictBundleBlock.status -ceq 'blocked-as-required' -and
    [string] $strictBundleBlock.releaseTag -ceq $ReleaseTag -and
    [string] $strictBundleBlock.sourceCommit -ceq $SourceCommit
) -Message 'Strict public bundle gate was not proven blocked for this tag and source commit.'
Assert-Condition -Condition ([string] $smokeReport.status -ceq 'passed') -Message 'Community Beta startup smoke did not pass.'

$mainPayloadEntries = @($bundleReport.nsisPayload.entries | Where-Object { [string] $_.destination -ceq 'valorant-highlight-manager.exe' })
Assert-Condition -Condition ($mainPayloadEntries.Count -eq 1) -Message 'Bundle report must identify exactly one embedded main executable.'
$verifiedMainHash = [string] $mainPayloadEntries[0].normalization.rawEmbeddedSha256
Assert-Condition -Condition (
    $verifiedMainHash -cmatch '^[0-9a-f]{64}$' -and
    $verifiedMainHash -ceq [string] $mainPayloadEntries[0].sha256 -and
    $verifiedMainHash -ceq [string] $smokeReport.executable.sha256
) -Message 'Startup smoke executable is not hash-bound to the bundle-gate main payload.'
Assert-Condition -Condition (
    [string] $bundleReport.ffmpegManifest.providerReleaseTag -ceq $ReleaseTag -and
    [string] $bundleReport.ffmpegManifest.sha256 -ceq (Get-Sha256 -Path $ffmpegManifestFile)
) -Message 'Bundle report is not bound to the exact staged FFmpeg manifest and release tag.'

$installerSignature = (Get-AuthenticodeSignature -LiteralPath $installer).Status.ToString()
Assert-Condition -Condition ($installerSignature -ceq 'NotSigned') -Message "Community Beta installer must be NotSigned; got '$installerSignature'."
Assert-Condition -Condition ((Get-Sha256 -Path $installer) -ceq [string] $bundleReport.artifacts.nsisInstaller.sha256) -Message 'Installer hash does not match the Community Beta bundle report.'

$sourceArchive = Get-PackageFile -Root $ffmpegPackage -RelativePath ([string] $technicalManifest.correspondingSource.path) -Description 'FFmpeg corresponding-source archive'
Assert-Condition -Condition (
    (Get-Sha256 -Path $sourceArchive) -ceq [string] $technicalManifest.correspondingSource.sha256 -and
    (Get-Item -LiteralPath $sourceArchive).Length -eq [long] $technicalManifest.correspondingSource.sizeBytes
) -Message 'FFmpeg source archive does not match the hash-bound technical package manifest.'
Assert-Condition -Condition (
    [string] $technicalManifest.sourceCommit -ceq [string] $manifest.ffmpeg.upstreamCommit -and
    [string] $technicalManifest.executable.sha256 -ceq [string] $manifest.artifact.executableSha256 -and
    [long] $technicalManifest.executable.sizeBytes -eq [long] $manifest.artifact.executableSizeBytes -and
    [string] $technicalManifest.correspondingSource.sha256 -ceq [string] $manifest.sourceCompliance.correspondingSourceBundle.sha256 -and
    [long] $technicalManifest.correspondingSource.sizeBytes -eq [long] $manifest.sourceCompliance.correspondingSourceBundle.sizeBytes
) -Message 'Staged FFmpeg manifest is not bound to the executable and source from the exact technical package.'
Assert-Condition -Condition ((Get-Sha256 -Path $ffmpegArchive) -ceq [string] $manifest.artifact.sha256 -and (Get-Item -LiteralPath $ffmpegArchive).Length -eq [long] $manifest.artifact.sizeBytes) -Message 'FFmpeg binary archive does not match the manifest.'
Assert-Condition -Condition ((Get-Sha256 -Path $sourceArchive) -ceq [string] $manifest.sourceCompliance.correspondingSourceBundle.sha256 -and (Get-Item -LiteralPath $sourceArchive).Length -eq [long] $manifest.sourceCompliance.correspondingSourceBundle.sizeBytes) -Message 'FFmpeg source archive does not match the manifest.'
Assert-Condition -Condition (
    [string] $manifest.artifact.fileName -ceq [System.IO.Path]::GetFileName($ffmpegArchive) -and
    [string] $bundleReport.communityBetaDecision.ffmpegBinaryArchive.sha256 -ceq [string] $manifest.artifact.sha256 -and
    [long] $bundleReport.communityBetaDecision.ffmpegBinaryArchive.sizeBytes -eq [long] $manifest.artifact.sizeBytes -and
    [string] $bundleReport.communityBetaDecision.ffmpegCorrespondingSource.sha256 -ceq [string] $manifest.sourceCompliance.correspondingSourceBundle.sha256 -and
    [long] $bundleReport.communityBetaDecision.ffmpegCorrespondingSource.sizeBytes -eq [long] $manifest.sourceCompliance.correspondingSourceBundle.sizeBytes -and
    [string] $bundleReport.artifacts.ffmpeg.sha256 -ceq [string] $manifest.artifact.executableSha256
) -Message 'Bundle report, staged FFmpeg manifest, binary archive, and corresponding-source sidecar are not hash-bound to one another.'

[void] [System.IO.Directory]::CreateDirectory($output)
$installerOutputName = "VALOFRAME-$ReleaseTag-x64-unsigned-setup.exe"
$installerOutputPath = Join-Path $output $installerOutputName
Copy-Item -LiteralPath $installer -Destination $installerOutputPath
$installerSha256 = Get-Sha256 -Path $installerOutputPath
Copy-Item -LiteralPath $ffmpegArchive -Destination (Join-Path $output ([System.IO.Path]::GetFileName($ffmpegArchive)))
Copy-Item -LiteralPath $sourceArchive -Destination (Join-Path $output ([System.IO.Path]::GetFileName($sourceArchive)))
Copy-Item -LiteralPath $bundleReportFile -Destination (Join-Path $output 'community-beta-bundle-report.json')
Copy-Item -LiteralPath $preflightReportFile -Destination (Join-Path $output 'strict-public-preflight.json')
Copy-Item -LiteralPath $strictBundleBlockFile -Destination (Join-Path $output 'strict-public-bundle-block.json')
Copy-Item -LiteralPath $smokeReportFile -Destination (Join-Path $output 'community-beta-smoke-report.json')

$complianceZipPath = Join-Path $output 'community-beta-compliance.zip'
Add-Type -AssemblyName System.IO.Compression
$zipStream = [System.IO.File]::Open($complianceZipPath, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None)
try {
    $zip = [System.IO.Compression.ZipArchive]::new($zipStream, [System.IO.Compression.ZipArchiveMode]::Create, $true)
    try {
        $licenseFiles = @(Get-ChildItem -LiteralPath $licenseRoot -File -Recurse -Force | Sort-Object FullName)
        Assert-Condition -Condition ($licenseFiles.Count -gt 10) -Message 'Compliance archive has an unexpectedly small license set.'
        foreach ($file in $licenseFiles) {
            Assert-Condition -Condition (($file.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -eq 0) -Message 'Compliance archive input contains a reparse point.'
            $relative = [System.IO.Path]::GetRelativePath($licenseRoot, $file.FullName).Replace('\', '/')
            Add-ZipEntry -Archive $zip -EntryPath "licenses/$relative" -SourcePath $file.FullName
        }
        Add-ZipEntry -Archive $zip -EntryPath 'ffmpeg-manifest.json' -SourcePath $ffmpegManifestFile
    }
    finally { $zip.Dispose() }
}
finally { $zipStream.Dispose() }

$notice = @"
UNSIGNED COMMUNITY BETA — NOT A FORMAL PUBLIC RELEASE

Release: $ReleaseTag
Source commit: $SourceCommit

This Windows installer is intentionally unsigned and has no automatic updater.
Windows may show Unknown Publisher or SmartScreen warnings. Verify SHA256SUMS.txt
before running it. FFmpeg is used only for local thumbnail generation; its exact
binary/build evidence and corresponding source are included beside the installer.

The strict public-release policy remains blocked. This prerelease does not claim
Authenticode identity, trusted timestamp, formal VM evidence, or legal approval.
"@
Write-LfText -Path (Join-Path $output 'COMMUNITY-BETA-NOTICE.txt') -Text $notice -Encoding ([System.Text.UTF8Encoding]::new($false))

$repositoryUrl = "https://github.com/$RepositorySlug"
$installerDownloadUrl = "$repositoryUrl/releases/download/$ReleaseTag/$installerOutputName"
$mirrorSection = if ($hasMirror) {
@"
### 备用镜像

[**下载 ``$MirrorFileName``**]($MirrorUrl)

- 密码：``$MirrorPassword``
- SHA-256：``$MirrorSha256``

> 镜像文件可能与 GitHub 安装包使用不同文件名或打包方式，请只使用这一节对应的 SHA-256 核验镜像文件，不要混用校验值。

"@
}
else { '' }

$releaseNotes = @"
# 瓦刻（VALOFRAME）$ReleaseTag · Community Beta

面向 Windows 10/11 x64 的社区测试版本。瓦刻在本机整理《无畏契约》高光视频，提供按账号与对局归组、筛选、预览、收藏、标签、备注、回收站和导出能力。

## 下载

### GitHub（推荐）

[**下载 ``$installerOutputName``**]($installerDownloadUrl)

- SHA-256：``$installerSha256``

$mirrorSection> 普通用户只需下载本节列出的 ``.exe`` 安装包。本页的 ``Source code``、FFmpeg 压缩包、许可归档、JSON 报告、校验清单等属于源码或技术合规附件，不是安装程序。

## 安装与 Windows 提示

1. 下载后运行 ``Get-FileHash -Algorithm SHA256 -LiteralPath '.\安装包文件名.exe'``，确认结果与所选入口公布的 SHA-256 完全一致。
2. 当前安装包未签名，Windows 可能显示“未知发布者”或 SmartScreen 的“Windows 已保护你的电脑”。确认来源和哈希后，可选择“更多信息”→“仍要运行”；无法确认时请取消安装。
3. 此版本没有自动更新，后续版本需要手动下载安装。

## 主要功能

- 自动发现并只读扫描本机高光来源，按账号与对局整理素材。
- 按英雄、地图、模式、日期、视频类型、标签和文件状态筛选。
- 本地预览、收藏、自定义标签、备注、回收站和批量导出。
- 本地优先：不提供云同步或遥测，视频与索引数据保留在设备上。

## 测试版说明

- 首次使用前建议备份应用数据；测试永久删除时只使用可丢弃的视频副本。
- FFmpeg 仅用于本地缩略图生成。最小二进制、构建证据、许可证和对应源码与安装包同时提供，普通安装无需单独下载这些技术附件。
- 这是未签名的 Community Beta，不是仓库定义的严格正式发布。代码签名、可信时间戳、正式 VM/数据安全证据和完整审阅仍在后续版本独立完成。
- 项目为非官方社区工具，与 Riot Games、腾讯及其关联公司不存在隶属、赞助或认可关系。

## 反馈

- [提交 Beta 反馈]($repositoryUrl/issues/new?template=beta_feedback.yml)
- [报告可复现问题]($repositoryUrl/issues/new?template=bug_report.yml)
- [阅读 Community Beta 完整说明]($repositoryUrl/blob/main/docs/COMMUNITY_BETA.md)

Source commit: ``$SourceCommit``
"@
Write-LfText -Path (Join-Path $output 'RELEASE-NOTES.md') -Text $releaseNotes -Encoding ([System.Text.UTF8Encoding]::new($false))

$allowedNames = @(
    $installerOutputName,
    'valoframe-ffmpeg-minimal-windows-x64.zip',
    'ffmpeg-corresponding-source.tar.xz',
    'community-beta-compliance.zip',
    'community-beta-bundle-report.json',
    'strict-public-preflight.json',
    'strict-public-bundle-block.json',
    'community-beta-smoke-report.json',
    'COMMUNITY-BETA-NOTICE.txt',
    'RELEASE-NOTES.md'
)
$releaseFiles = @(Get-ChildItem -LiteralPath $output -File -Force | Sort-Object Name)
Assert-Condition -Condition ($releaseFiles.Count -eq $allowedNames.Count -and @($releaseFiles | Where-Object { $_.Name -cnotin $allowedNames }).Count -eq 0) -Message 'Community Beta release asset set is not exact.'
$checksumLines = foreach ($file in $releaseFiles) {
    '{0}  {1}' -f (Get-Sha256 -Path $file.FullName), $file.Name
}
Write-LfText -Path (Join-Path $output 'SHA256SUMS.txt') -Text ($checksumLines -join "`n") -Encoding ([System.Text.ASCIIEncoding]::new())

[ordered]@{
    schemaVersion = 1
    status = 'community-beta-release-assets-ready'
    strictPublicReleaseApproved = $false
    releaseTag = $ReleaseTag
    sourceCommit = $SourceCommit
    outputDirectory = $output
    fileCount = $allowedNames.Count + 1
    installer = [ordered]@{
        fileName = $installerOutputName
        sha256 = $installerSha256
        authenticodeStatus = $installerSignature
    }
    checksumManifest = 'SHA256SUMS.txt'
} | ConvertTo-Json -Depth 6
