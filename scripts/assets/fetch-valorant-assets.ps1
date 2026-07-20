[CmdletBinding()]
param(
    [Parameter()]
    [switch] $Refresh
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repositoryRoot = [System.IO.Path]::GetFullPath((Split-Path -Parent (Split-Path -Parent $PSScriptRoot)))
$manifestPath = Join-Path $repositoryRoot 'src\data\valorantAssets.json'
$verifierPath = Join-Path $PSScriptRoot 'verify-valorant-assets.mjs'
$manifest = Get-Content -Raw -LiteralPath $manifestPath -Encoding UTF8 | ConvertFrom-Json -Depth 100

if ([long] $manifest.schemaVersion -ne 2) {
    throw 'The VALORANT artwork manifest must use schemaVersion 2.'
}
if ([string] $manifest.sourceBase -cne 'https://media.valorant-api.com') {
    throw 'The VALORANT artwork manifest has an unexpected sourceBase.'
}

function Resolve-SafeChildPath {
    param(
        [Parameter(Mandatory)] [string] $Root,
        [Parameter(Mandatory)] [string] $RelativePath,
        [Parameter(Mandatory)] [string] $Description
    )

    if ([System.IO.Path]::IsPathRooted($RelativePath) -or $RelativePath.Contains('\') -or
        $RelativePath.Contains(':') -or $RelativePath.IndexOf([char] 0) -ge 0) {
        throw "$Description is not a safe relative POSIX path: '$RelativePath'."
    }
    $segments = $RelativePath.Split('/')
    if (@($segments | Where-Object { [string]::IsNullOrWhiteSpace($_) -or $_ -in @('.', '..') }).Count -ne 0) {
        throw "$Description contains an unsafe path segment: '$RelativePath'."
    }
    $rootFull = [System.IO.Path]::GetFullPath($Root)
    $candidate = [System.IO.Path]::GetFullPath((Join-Path $rootFull ($segments -join [System.IO.Path]::DirectorySeparatorChar)))
    $prefix = $rootFull.TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
    if (-not $candidate.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "$Description escapes its approved root: '$RelativePath'."
    }
    return $candidate
}

function Assert-NoReparseAncestors {
    param(
        [Parameter(Mandatory)] [string] $Root,
        [Parameter(Mandatory)] [string] $LiteralPath,
        [Parameter(Mandatory)] [string] $Description
    )

    $rootFull = [System.IO.Path]::GetFullPath($Root)
    $pathFull = [System.IO.Path]::GetFullPath($LiteralPath)
    $relativePath = [System.IO.Path]::GetRelativePath($rootFull, $pathFull)
    if ($relativePath -eq '..' -or $relativePath.StartsWith(('..' + [System.IO.Path]::DirectorySeparatorChar)) -or
        [System.IO.Path]::IsPathRooted($relativePath)) {
        throw "$Description escapes the validated root."
    }

    $rootItem = Get-Item -LiteralPath $rootFull -Force
    if (($rootItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$Description uses a reparse-point root: '$rootFull'."
    }
    $current = $rootFull
    foreach ($segment in @($relativePath.Split(
                [System.IO.Path]::DirectorySeparatorChar,
                [System.StringSplitOptions]::RemoveEmptyEntries
            ))) {
        $current = Join-Path $current $segment
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
            if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "$Description contains a reparse point: '$current'."
            }
        }
    }
}

function Assert-ApprovedSourceUri {
    param(
        [Parameter(Mandatory)] [System.Uri] $Uri
    )

    if (-not $Uri.IsAbsoluteUri -or $Uri.Scheme -cne 'https' -or
        $Uri.Host -cne 'media.valorant-api.com' -or -not $Uri.IsDefaultPort) {
        throw "Asset source URL is outside the approved HTTPS origin: '$Uri'."
    }
}

function Receive-PinnedAsset {
    param(
        [Parameter(Mandatory)] [System.Net.Http.HttpClient] $Client,
        [Parameter(Mandatory)] [System.Uri] $SourceUri,
        [Parameter(Mandatory)] [string] $DestinationPath,
        [Parameter(Mandatory)] [long] $ExpectedBytes
    )

    $currentUri = $SourceUri
    for ($redirectCount = 0; $redirectCount -le 3; $redirectCount++) {
        Assert-ApprovedSourceUri -Uri $currentUri
        $request = [System.Net.Http.HttpRequestMessage]::new([System.Net.Http.HttpMethod]::Get, $currentUri)
        $response = $null
        $requestDeadline = [System.Threading.CancellationTokenSource]::new()
        $requestDeadline.CancelAfter([System.TimeSpan]::FromSeconds(30))
        try {
            $response = $Client.SendAsync(
                $request,
                [System.Net.Http.HttpCompletionOption]::ResponseHeadersRead,
                $requestDeadline.Token
            ).GetAwaiter().GetResult()

            if ([int] $response.StatusCode -in @(301, 302, 303, 307, 308)) {
                if ($redirectCount -eq 3 -or $null -eq $response.Headers.Location) {
                    throw "Asset response exceeded the approved redirect limit: '$SourceUri'."
                }
                $nextUri = if ($response.Headers.Location.IsAbsoluteUri) {
                    $response.Headers.Location
                }
                else {
                    [System.Uri]::new($currentUri, $response.Headers.Location)
                }
                Assert-ApprovedSourceUri -Uri $nextUri
                $currentUri = $nextUri
                continue
            }
            if ($response.StatusCode -ne [System.Net.HttpStatusCode]::OK) {
                throw "Unexpected HTTP status $([int] $response.StatusCode) for '$currentUri'."
            }

            $mediaType = [string] $response.Content.Headers.ContentType.MediaType
            if ($mediaType -ine 'image/png') {
                throw "Unexpected Content-Type '$mediaType' for '$currentUri'."
            }
            $declaredLength = $response.Content.Headers.ContentLength
            if ($null -ne $declaredLength -and [long] $declaredLength -gt $ExpectedBytes) {
                throw "Asset response exceeds the manifest byte length: '$currentUri'."
            }

            $input = $response.Content.ReadAsStreamAsync($requestDeadline.Token).GetAwaiter().GetResult()
            $output = [System.IO.File]::Open(
                $DestinationPath,
                [System.IO.FileMode]::CreateNew,
                [System.IO.FileAccess]::Write,
                [System.IO.FileShare]::None
            )
            try {
                $buffer = [byte[]]::new(65536)
                $total = [long] 0
                while (($read = $input.ReadAsync(
                            $buffer,
                            0,
                            $buffer.Length,
                            $requestDeadline.Token
                        ).GetAwaiter().GetResult()) -gt 0) {
                    if ($total + $read -gt $ExpectedBytes) {
                        throw "Asset response exceeds the manifest byte length: '$currentUri'."
                    }
                    $output.Write($buffer, 0, $read)
                    $total += $read
                }
                if ($total -ne $ExpectedBytes) {
                    throw "Asset response byte length does not match the manifest: '$currentUri'."
                }
            }
            finally {
                $output.Dispose()
                $input.Dispose()
            }
            return
        }
        finally {
            $request.Dispose()
            if ($null -ne $response) {
                $response.Dispose()
            }
            $requestDeadline.Dispose()
        }
    }
}

function Assert-DownloadedAsset {
    param(
        [Parameter(Mandatory)] [string] $LiteralPath,
        [Parameter(Mandatory)] [object] $ManifestEntry
    )

    $item = Get-Item -LiteralPath $LiteralPath -Force
    if ($item.PSIsContainer -or ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Downloaded asset is not a regular file: '$LiteralPath'."
    }
    if ([long] $item.Length -ne [long] $ManifestEntry.byteLength) {
        throw "Downloaded asset byte length does not match the manifest: '$($ManifestEntry.relativePath)'."
    }
    $actualHash = (Get-FileHash -LiteralPath $LiteralPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -cne [string] $ManifestEntry.sha256) {
        throw "Downloaded asset SHA-256 does not match the manifest: '$($ManifestEntry.relativePath)'."
    }

    $stream = [System.IO.File]::OpenRead($LiteralPath)
    try {
        $header = [byte[]]::new(24)
        if ($stream.Read($header, 0, $header.Length) -ne $header.Length) {
            throw "Downloaded asset is too short to be a PNG: '$($ManifestEntry.relativePath)'."
        }
    }
    finally {
        $stream.Dispose()
    }
    $signature = [byte[]](137, 80, 78, 71, 13, 10, 26, 10)
    for ($index = 0; $index -lt $signature.Length; $index++) {
        if ($header[$index] -ne $signature[$index]) {
            throw "Downloaded asset has an invalid PNG signature: '$($ManifestEntry.relativePath)'."
        }
    }
    $width = [System.Net.IPAddress]::NetworkToHostOrder([System.BitConverter]::ToInt32($header, 16))
    $height = [System.Net.IPAddress]::NetworkToHostOrder([System.BitConverter]::ToInt32($header, 20))
    if ($width -ne [long] $ManifestEntry.width -or $height -ne [long] $ManifestEntry.height) {
        throw "Downloaded asset dimensions do not match the manifest: '$($ManifestEntry.relativePath)'."
    }
}

$outputRoot = Resolve-SafeChildPath `
    -Root $repositoryRoot `
    -RelativePath ([string] $manifest.assetRoot) `
    -Description 'VALORANT artwork output root'
Assert-NoReparseAncestors -Root $repositoryRoot -LiteralPath $outputRoot -Description 'VALORANT artwork output root'

if (-not $Refresh) {
    & node $verifierPath --repository-root $repositoryRoot
    if ($LASTEXITCODE -ne 0) {
        throw 'The existing VALORANT artwork set failed verification. Use -Refresh only with the pinned manifest snapshot after manual scope review.'
    }
    return
}

& node $verifierPath --repository-root $repositoryRoot --metadata-only --quiet
if ($LASTEXITCODE -ne 0) {
    throw 'The manifest and authorization record must pass metadata verification before any download.'
}

$stagingBase = Join-Path $repositoryRoot '.tmp'
New-Item -ItemType Directory -Path $stagingBase -Force | Out-Null
Assert-NoReparseAncestors -Root $repositoryRoot -LiteralPath $stagingBase -Description 'asset staging base'
$stagingLeaf = 'valorant-assets-refresh-{0}' -f [System.Guid]::NewGuid().ToString('N')
$stagingRoot = Join-Path $stagingBase $stagingLeaf
New-Item -ItemType Directory -Path $stagingRoot | Out-Null
Assert-NoReparseAncestors -Root $repositoryRoot -LiteralPath $stagingRoot -Description 'asset staging root'

$handler = $null
$client = $null
try {
    $handler = [System.Net.Http.HttpClientHandler]::new()
    $handler.AllowAutoRedirect = $false
    $client = [System.Net.Http.HttpClient]::new($handler)
    $client.Timeout = [System.TimeSpan]::FromSeconds(30)
    $client.DefaultRequestHeaders.UserAgent.ParseAdd('VALOFRAME-asset-refresh/1.0')

    foreach ($category in @('agents', 'maps')) {
        foreach ($asset in @($manifest.$category)) {
            $expectedRelativePath = '{0}/{1}.png' -f $category, $asset.uuid
            if ([string] $asset.relativePath -cne $expectedRelativePath) {
                throw "Manifest relativePath does not match its category and UUID: '$($asset.relativePath)'."
            }

            $sourceUri = [System.Uri]([string] $asset.sourceUrl)
            Assert-ApprovedSourceUri -Uri $sourceUri
            $stagedPath = Resolve-SafeChildPath `
                -Root $stagingRoot `
                -RelativePath ([string] $asset.relativePath) `
                -Description 'staged asset'
            $stagedDirectory = Split-Path -Parent $stagedPath
            New-Item -ItemType Directory -Path $stagedDirectory -Force | Out-Null
            Assert-NoReparseAncestors -Root $repositoryRoot -LiteralPath $stagedDirectory -Description 'staged asset directory'
            $temporaryPath = '{0}.download' -f $stagedPath

            $lastError = $null
            for ($attempt = 1; $attempt -le 3; $attempt++) {
                try {
                    Receive-PinnedAsset `
                        -Client $client `
                        -SourceUri $sourceUri `
                        -DestinationPath $temporaryPath `
                        -ExpectedBytes ([long] $asset.byteLength)
                    Assert-DownloadedAsset -LiteralPath $temporaryPath -ManifestEntry $asset
                    Move-Item -LiteralPath $temporaryPath -Destination $stagedPath
                    $lastError = $null
                    break
                }
                catch {
                    $lastError = $_
                    if (Test-Path -LiteralPath $temporaryPath -PathType Leaf) {
                        Remove-Item -LiteralPath $temporaryPath -Force
                    }
                    if ($attempt -lt 3) {
                        Start-Sleep -Seconds $attempt
                    }
                }
            }
            if ($null -ne $lastError) {
                throw $lastError
            }
        }
    }

    $stagingRelativePath = [System.IO.Path]::GetRelativePath($repositoryRoot, $stagingRoot).Replace('\', '/')
    & node `
        $verifierPath `
        --repository-root $repositoryRoot `
        --asset-root $stagingRelativePath `
        --quiet
    if ($LASTEXITCODE -ne 0) {
        throw 'The complete staged VALORANT artwork set failed verification; no pinned file was replaced.'
    }

    foreach ($asset in @($manifest.agents) + @($manifest.maps)) {
        $stagedPath = Resolve-SafeChildPath `
            -Root $stagingRoot `
            -RelativePath ([string] $asset.relativePath) `
            -Description 'staged asset'
        $outputPath = Resolve-SafeChildPath `
            -Root $outputRoot `
            -RelativePath ([string] $asset.relativePath) `
            -Description 'installed asset'
        $outputDirectory = Split-Path -Parent $outputPath
        New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
        Assert-NoReparseAncestors -Root $repositoryRoot -LiteralPath $outputPath -Description 'installed asset path'
        Move-Item -LiteralPath $stagedPath -Destination $outputPath -Force
    }

    & node $verifierPath --repository-root $repositoryRoot
    if ($LASTEXITCODE -ne 0) {
        throw 'The installed VALORANT artwork set failed final verification.'
    }
}
finally {
    if ($null -ne $client) {
        $client.Dispose()
    }
    elseif ($null -ne $handler) {
        $handler.Dispose()
    }
    if (Test-Path -LiteralPath $stagingRoot) {
        $stagingFull = [System.IO.Path]::GetFullPath($stagingRoot)
        $stagingPrefix = [System.IO.Path]::GetFullPath($stagingBase).TrimEnd('\', '/') +
            [System.IO.Path]::DirectorySeparatorChar
        if (-not $stagingFull.StartsWith($stagingPrefix, [System.StringComparison]::OrdinalIgnoreCase) -or
            (Split-Path -Leaf $stagingFull) -cnotmatch '^valorant-assets-refresh-[0-9a-f]{32}$') {
            throw "Refusing to clean an unexpected staging path: '$stagingFull'."
        }
        Assert-NoReparseAncestors -Root $repositoryRoot -LiteralPath $stagingFull -Description 'asset staging cleanup'
        Remove-Item -LiteralPath $stagingFull -Recurse -Force
    }
}
