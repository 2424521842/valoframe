#requires -Version 7.2

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$preflightScript = Join-Path $repositoryRoot 'scripts\release\public-release-preflight.ps1'
$stagingScript = Join-Path $repositoryRoot 'scripts\release\stage-public-release-evidence.ps1'
$fixtureParent = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd('\', '/')
$fixtureRoot = Join-Path $fixtureParent ('vhm-public-release-fixture-' + [Guid]::NewGuid().ToString('N'))
$fixtureRepository = Join-Path $fixtureRoot 'repository'
$evidenceSource = Join-Path $fixtureRoot 'evidence-source'
$stagedEvidence = Join-Path $fixtureRoot 'vhm-public-release-evidence-staged'
$evidenceArchive = Join-Path $fixtureRoot 'protected-evidence.zip'
$sourceCommit = '1234567890abcdef1234567890abcdef12345678'

function Write-Utf8File {
    param(
        [Parameter(Mandatory)] [string] $LiteralPath,
        [Parameter(Mandatory)] [AllowEmptyString()] [string] $Text
    )
    $parent = [System.IO.Directory]::GetParent([System.IO.Path]::GetFullPath($LiteralPath))
    [void] [System.IO.Directory]::CreateDirectory($parent.FullName)
    [System.IO.File]::WriteAllText($LiteralPath, $Text, [System.Text.UTF8Encoding]::new($false))
}

function Write-JsonFile {
    param(
        [Parameter(Mandatory)] [string] $LiteralPath,
        [Parameter(Mandatory)] [object] $Value
    )
    Write-Utf8File -LiteralPath $LiteralPath -Text (($Value | ConvertTo-Json -Depth 100) + "`n")
}

function Get-Hash {
    param([Parameter(Mandatory)] [string] $LiteralPath)
    return (Get-FileHash -LiteralPath $LiteralPath -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Assert-True {
    param(
        [Parameter(Mandatory)] [bool] $Condition,
        [Parameter(Mandatory)] [string] $Message
    )
    if (-not $Condition) { throw $Message }
}

function Invoke-FixturePreflight {
    $json = & $preflightScript `
        -RepositoryRoot $fixtureRepository `
        -PolicyPath (Join-Path $fixtureRepository 'release\public-release-policy.json') `
        -ExpectedSourceCommit $sourceCommit `
        -EvidenceRoot $stagedEvidence
    return (($json -join "`n") | ConvertFrom-Json -Depth 100)
}

try {
    [void] [System.IO.Directory]::CreateDirectory($fixtureRepository)
    [void] [System.IO.Directory]::CreateDirectory($evidenceSource)

    $licensePath = Join-Path $fixtureRepository 'LICENSE'
    $scopePath = Join-Path $fixtureRepository 'LICENSE-SCOPE.txt'
    Write-Utf8File -LiteralPath $licensePath -Text "MIT fixture license`n"
    Write-Utf8File -LiteralPath $scopePath -Text "Fixture scope`n"
    Write-JsonFile -LiteralPath (Join-Path $fixtureRepository 'package.json') -Value ([ordered]@{
            name = 'fixture'
            version = '0.2.0'
            license = 'MIT'
        })
    Write-Utf8File -LiteralPath (Join-Path $fixtureRepository 'src-tauri\Cargo.toml') -Text "[package]`nname = `"fixture`"`nversion = `"0.2.0`"`nlicense = `"MIT`"`n"
    Write-JsonFile -LiteralPath (Join-Path $fixtureRepository 'src-tauri\tauri.conf.json') -Value ([ordered]@{
            productName = 'Fixture'
            version = '0.2.0'
            identifier = 'com.example.fixture'
            bundle = [ordered]@{
                resources = [ordered]@{
                    '../LICENSE' = 'licenses/project/LICENSE.txt'
                    '../LICENSE-SCOPE.txt' = 'licenses/project/LICENSE-SCOPE.txt'
                }
            }
        })

    $complianceRoot = Join-Path $fixtureRepository 'generated\third-party'
    foreach ($name in @(
            'COMPLIANCE-MANIFEST.json',
            'THIRD-PARTY-NOTICES.md',
            'THIRD-PARTY-LICENSES.txt',
            'npm-runtime.spdx.json',
            'cargo-windows-x64.spdx.json')) {
        Write-Utf8File -LiteralPath (Join-Path $complianceRoot $name) -Text "fixture $name`n"
    }
    Write-JsonFile -LiteralPath (Join-Path $complianceRoot 'COMPLIANCE-SUMMARY.json') -Value ([ordered]@{
            schemaVersion = 1
            publicRedistributionReady = $true
            blockers = @()
        })

    Write-JsonFile -LiteralPath (Join-Path $fixtureRepository 'third_party\ffmpeg\windows-x64.json') -Value ([ordered]@{
            sourceCompliance = [ordered]@{
                redistributionReady = $true
                status = 'ready-for-redistribution'
            }
        })
    Write-Utf8File -LiteralPath (Join-Path $fixtureRepository 'docs\PUBLIC-DISCLAIMER.md') -Text "Synthetic fixture disclaimer.`n"
    Write-Utf8File -LiteralPath (Join-Path $fixtureRepository 'scripts\assets\verify-fixture.mjs') -Text "process.exit(0);`n"

    $gameApprovalRelative = 'release/approvals/game-content-rights.json'
    $gameManifestPath = Join-Path $fixtureRepository 'src\data\valorantAssets.json'
    Write-JsonFile -LiteralPath $gameManifestPath -Value ([ordered]@{
            schemaVersion = 1
            authorizationReference = $gameApprovalRelative
            assets = @()
        })
    $gameManifestHash = Get-Hash -LiteralPath $gameManifestPath
    $publicScopes = @(
        'public-source-repository',
        'in-app-display',
        'public-windows-installer',
        'public-release-artifact-download',
        'github-project-marketing'
    )
    $operationalScopes = @(
        'public-source-repository',
        'in-app-display',
        'internal-controlled-testing',
        'windows-internal-test-build',
        'github-project-marketing'
    )
    $approvedGameRecord = [ordered]@{
        schemaVersion = 1
        status = 'approved-for-public-release'
        approved = $true
        ownerAttestationReceived = $true
        sourceDocumentReviewed = $true
        legalReviewApproved = $true
        manualReviewRequired = $false
        evidenceReference = 'synthetic-fixture-only'
        evidenceDocumentSha256 = ('a' * 64)
        rightsHolderIdentity = 'Synthetic Fixture Rights Holder'
        licensee = 'Synthetic Fixture Licensee'
        approvedScopes = $publicScopes
        repositoryOperationalAssumptionScopes = $operationalScopes
        assetSet = [ordered]@{
            manifestSha256 = $gameManifestHash
        }
    }
    $gameApprovalPath = Join-Path $fixtureRepository ($gameApprovalRelative.Replace('/', '\'))
    Write-JsonFile -LiteralPath $gameApprovalPath -Value $approvedGameRecord

    $cleanVmCodes = @(
        'fresh-install-webview2-present',
        'fresh-install-webview2-missing-online',
        'fresh-install-webview2-missing-offline',
        'windows-10-x64',
        'windows-11-x64',
        'dpi-100-percent',
        'dpi-150-percent',
        'dpi-200-percent',
        'minimum-window-760x560',
        'nvidia-real-output-import-rescan-startup-preview-review',
        'tracker-real-output-import-rescan-startup-preview-review',
        'same-source-subdirectory-rename-auto-reconnect-user-state-preserved',
        'source-root-relocation-user-state-preserved',
        'kill-death-timeline-icons-tooltips-accessibility-and-seek',
        'same-version-reinstall',
        'v0.1.0-beta.1-manual-upgrade-to-v0.2.1',
        'signed-updater-v0.2.1-to-v0.2.2-schema-v18-user-state-preserved',
        'signed-updater-upgrade-to-higher-patch',
        'downgrade-rejected',
        'uninstall-user-data-preserved',
        'packaged-ffmpeg-only'
    )
    $dataSafetyCodes = @(
        'source-media-readonly-default',
        'index-only-removal-source-media-sha256-unchanged',
        'permanent-delete-explicit-confirmation',
        'application-data-boundary',
        'uninstall-user-data-preserved'
    )
    function New-EvidenceRecords {
        param(
            [Parameter(Mandatory)] [string] $Prefix,
            [Parameter(Mandatory)] [string[]] $Codes
        )
        return @($Codes | ForEach-Object {
                $relative = "$Prefix/$_.txt"
                $path = Join-Path $evidenceSource ($relative.Replace('/', '\'))
                Write-Utf8File -LiteralPath $path -Text "Synthetic fixture evidence for $_ only.`n"
                [ordered]@{
                    code = $_
                    status = 'passed'
                    evidencePath = $relative
                    sha256 = Get-Hash -LiteralPath $path
                }
            })
    }
    Write-JsonFile -LiteralPath (Join-Path $evidenceSource 'clean-vm-evidence.json') -Value ([ordered]@{
            schemaVersion = 1
            sourceCommit = $sourceCommit
            approved = $true
            approvalReference = 'synthetic-fixture-only'
            scenarios = New-EvidenceRecords -Prefix 'clean-vm' -Codes $cleanVmCodes
        })
    Write-JsonFile -LiteralPath (Join-Path $evidenceSource 'data-safety-evidence.json') -Value ([ordered]@{
            schemaVersion = 1
            sourceCommit = $sourceCommit
            approved = $true
            approvalReference = 'synthetic-fixture-only'
            checks = New-EvidenceRecords -Prefix 'data-safety' -Codes $dataSafetyCodes
        })

    $policy = [ordered]@{
        schemaVersion = 1
        releaseMode = 'public'
        projectLicense = [ordered]@{
            approved = $true
            spdxExpression = 'MIT'
            file = 'LICENSE'
            sha256 = Get-Hash -LiteralPath $licensePath
            scopeFile = 'LICENSE-SCOPE.txt'
            scopeSha256 = Get-Hash -LiteralPath $scopePath
            approvalReference = 'synthetic-fixture-only'
        }
        eula = [ordered]@{
            required = $false
            approved = $true
            file = $null
            approvalReference = 'synthetic-fixture-only'
        }
        thirdPartyCompliance = [ordered]@{
            approved = $true
            root = 'generated/third-party'
            manifest = 'COMPLIANCE-MANIFEST.json'
            summary = 'COMPLIANCE-SUMMARY.json'
            notices = 'THIRD-PARTY-NOTICES.md'
            licenseTexts = 'THIRD-PARTY-LICENSES.txt'
            npmRuntimeSbom = 'npm-runtime.spdx.json'
            cargoWindowsX64Sbom = 'cargo-windows-x64.spdx.json'
            approvalReference = 'synthetic-fixture-only'
        }
        identity = [ordered]@{
            productName = 'Fixture'
            brandApproved = $true
            brandApprovalReference = 'synthetic-fixture-only'
            publisherSubject = 'CN=Synthetic Fixture Publisher'
            publisherApproved = $true
            publisherApprovalReference = 'synthetic-fixture-only'
            identifier = 'com.example.fixture'
            identifierApproved = $true
            identifierApprovalReference = 'synthetic-fixture-only'
        }
        gameContentRights = [ordered]@{
            approved = $true
            confirmedScopes = $publicScopes
            operationalAssumptionScopes = $operationalScopes
            requiredScopes = $publicScopes
            manifest = 'src/data/valorantAssets.json'
            manifestSha256 = $gameManifestHash
            assetRoot = 'public/fixture-assets'
            verifier = 'scripts/assets/verify-fixture.mjs'
            approvalReference = $gameApprovalRelative
            approvalSha256 = Get-Hash -LiteralPath $gameApprovalPath
        }
        iconRights = [ordered]@{
            approved = $true
            confirmedScopes = @('windows-installer', 'public-download')
            requiredScopes = @('windows-installer', 'public-download')
            approvalReference = 'synthetic-fixture-only'
        }
        riotTencentDisclaimer = [ordered]@{
            approved = $true
            materialPath = 'docs/PUBLIC-DISCLAIMER.md'
            approvalReference = 'synthetic-fixture-only'
        }
        ffmpeg = [ordered]@{ manifest = 'third_party/ffmpeg/windows-x64.json' }
        authenticode = [ordered]@{
            certificateProvisioned = $true
            expectedPublisherSubject = 'CN=Synthetic Fixture Publisher'
            expectedCertificateThumbprint = ('B' * 40)
            trustedTimestampRequired = $true
            timestampUrl = 'https://timestamp.invalid/fixture'
            signtoolVerificationRequired = $true
            approvalReference = 'synthetic-fixture-only'
        }
        cleanVmValidation = [ordered]@{
            evidenceSource = 'protected-external-archive'
            evidenceManifest = 'clean-vm-evidence.json'
            requiredScenarios = $cleanVmCodes
        }
        updater = [ordered]@{
            decision = 'enabled'
            approvalReference = 'synthetic-fixture-only'
            endpoint = 'https://github.com/2424521842/valoframe/releases/latest/download/latest.json'
            publicKeyReference = 'release/updater-public.key'
        }
        dataSafety = [ordered]@{
            evidenceSource = 'protected-external-archive'
            evidenceManifest = 'data-safety-evidence.json'
            requiredChecks = $dataSafetyCodes
        }
    }
    $policyPath = Join-Path $fixtureRepository 'release\public-release-policy.json'
    Write-JsonFile -LiteralPath $policyPath -Value $policy

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [System.IO.Compression.ZipFile]::CreateFromDirectory($evidenceSource, $evidenceArchive)
    $stageJson = & $stagingScript `
        -ArchivePath $evidenceArchive `
        -ExpectedArchiveSha256 (Get-Hash -LiteralPath $evidenceArchive) `
        -ExpectedSourceCommit $sourceCommit `
        -OutputDirectory $stagedEvidence `
        -PolicyPath $policyPath
    $stage = ($stageJson -join "`n") | ConvertFrom-Json -Depth 100
    Assert-True -Condition ($stage.status -ceq 'staged') -Message 'Approved fixture evidence archive did not stage.'
    Assert-True -Condition ($stage.sourceCommit -ceq $sourceCommit) -Message 'Approved fixture evidence did not bind its source commit.'

    $approved = Invoke-FixturePreflight
    Assert-True -Condition ($approved.status -ceq 'passed' -and [int] $approved.blockerCount -eq 0) -Message 'Complete approved fixture should pass strict preflight.'

    $pendingRecord = [ordered]@{
        schemaVersion = 1
        status = 'owner-attested-pending-source-evidence-review'
        ownerAttestationReceived = $true
        sourceDocumentReviewed = $false
        legalReviewApproved = $false
        manualReviewRequired = $true
        repositoryOperationalAssumptionScopes = $operationalScopes
    }
    Write-JsonFile -LiteralPath $gameApprovalPath -Value $pendingRecord
    $policy.gameContentRights.approved = $false
    $policy.gameContentRights.confirmedScopes = @()
    $policy.gameContentRights.approvalSha256 = Get-Hash -LiteralPath $gameApprovalPath
    Write-JsonFile -LiteralPath $policyPath -Value $policy
    $pending = Invoke-FixturePreflight
    $pendingCodes = @($pending.blockers | ForEach-Object code)
    Assert-True -Condition ($pending.status -ceq 'blocked') -Message 'Pending game-content fixture must stay blocked.'
    Assert-True -Condition ('GAME_CONTENT_DISTRIBUTION_RIGHTS_MISSING' -cin $pendingCodes) -Message 'Pending game-content fixture did not report the distribution-rights blocker.'
    Assert-True -Condition ('GAME_CONTENT_EVIDENCE_INVALID' -cnotin $pendingCodes) -Message 'A legitimate pending game-content record was incorrectly treated as malformed.'

    Write-JsonFile -LiteralPath $gameApprovalPath -Value $approvedGameRecord
    $policy.gameContentRights.approved = $true
    $policy.gameContentRights.confirmedScopes = $publicScopes
    $policy.gameContentRights.approvalSha256 = Get-Hash -LiteralPath $gameApprovalPath
    Write-JsonFile -LiteralPath $policyPath -Value $policy
    $cleanManifestPath = Join-Path $stagedEvidence 'clean-vm-evidence.json'
    $dataManifestPath = Join-Path $stagedEvidence 'data-safety-evidence.json'
    $cleanManifestOriginalText = Get-Content -Raw -LiteralPath $cleanManifestPath -Encoding UTF8
    $dataManifestOriginalText = Get-Content -Raw -LiteralPath $dataManifestPath -Encoding UTF8
    $cleanManifest = $cleanManifestOriginalText | ConvertFrom-Json -Depth 100
    $cleanManifest.sourceCommit = 'abcdef1234567890abcdef1234567890abcdef12'
    Write-JsonFile -LiteralPath $cleanManifestPath -Value $cleanManifest
    $wrongCommit = Invoke-FixturePreflight
    $wrongCommitCodes = @($wrongCommit.blockers | ForEach-Object code)
    Assert-True -Condition ('CLEAN_VM_EVIDENCE_INVALID' -cin $wrongCommitCodes) -Message 'Evidence bound to a different source commit was not rejected.'

    Write-Utf8File -LiteralPath $cleanManifestPath -Text $cleanManifestOriginalText
    $cleanManifest = $cleanManifestOriginalText | ConvertFrom-Json -Depth 100
    $cleanManifest.scenarios = @($cleanManifest.scenarios | Where-Object { [string] $_.code -cne 'windows-11-x64' })
    Write-JsonFile -LiteralPath $cleanManifestPath -Value $cleanManifest
    $missingScenario = Invoke-FixturePreflight
    $missingScenarioCodes = @($missingScenario.blockers | ForEach-Object code)
    Assert-True -Condition ('CLEAN_VM_EVIDENCE_INVALID' -cin $missingScenarioCodes) -Message 'Clean-VM evidence missing the Windows 11 scenario was not rejected.'

    $v021CleanVmCodes = @(
        'same-source-subdirectory-rename-auto-reconnect-user-state-preserved',
        'source-root-relocation-user-state-preserved',
        'kill-death-timeline-icons-tooltips-accessibility-and-seek',
        'signed-updater-v0.2.1-to-v0.2.2-schema-v18-user-state-preserved'
    )
    foreach ($requiredCode in $v021CleanVmCodes) {
        Write-Utf8File -LiteralPath $cleanManifestPath -Text $cleanManifestOriginalText
        $cleanManifest = $cleanManifestOriginalText | ConvertFrom-Json -Depth 100
        $cleanManifest.scenarios = @($cleanManifest.scenarios | Where-Object { [string] $_.code -cne $requiredCode })
        Write-JsonFile -LiteralPath $cleanManifestPath -Value $cleanManifest
        $missingV021Scenario = Invoke-FixturePreflight
        $missingV021ScenarioCodes = @($missingV021Scenario.blockers | ForEach-Object code)
        Assert-True `
            -Condition ('CLEAN_VM_EVIDENCE_INVALID' -cin $missingV021ScenarioCodes) `
            -Message "Clean-VM evidence missing required v0.2.1 scenario '$requiredCode' was not rejected."
    }
    Write-Utf8File -LiteralPath $cleanManifestPath -Text $cleanManifestOriginalText

    $dataManifest = $dataManifestOriginalText | ConvertFrom-Json -Depth 100
    $dataManifest.checks = @($dataManifest.checks | Where-Object { [string] $_.code -cne 'index-only-removal-source-media-sha256-unchanged' })
    Write-JsonFile -LiteralPath $dataManifestPath -Value $dataManifest
    $missingIndexOnlySafety = Invoke-FixturePreflight
    $missingIndexOnlySafetyCodes = @($missingIndexOnlySafety.blockers | ForEach-Object code)
    Assert-True `
        -Condition ('DATA_SAFETY_EVIDENCE_INVALID' -cin $missingIndexOnlySafetyCodes) `
        -Message 'Data-safety evidence missing the index-only source-media SHA-256 check was not rejected.'
    Write-Utf8File -LiteralPath $dataManifestPath -Text $dataManifestOriginalText

    $unsafeArchive = Join-Path $fixtureRoot 'unsafe-evidence.zip'
    $unsafeStream = [System.IO.File]::Open($unsafeArchive, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None)
    $unsafeZip = [System.IO.Compression.ZipArchive]::new($unsafeStream, [System.IO.Compression.ZipArchiveMode]::Create, $false)
    try {
        $entry = $unsafeZip.CreateEntry('../escape.txt')
        $writer = [System.IO.StreamWriter]::new($entry.Open(), [System.Text.UTF8Encoding]::new($false))
        try { $writer.Write('escape') } finally { $writer.Dispose() }
    }
    finally {
        $unsafeZip.Dispose()
        $unsafeStream.Dispose()
    }
    $unsafeRejected = $false
    try {
        & $stagingScript `
            -ArchivePath $unsafeArchive `
            -ExpectedArchiveSha256 (Get-Hash -LiteralPath $unsafeArchive) `
            -ExpectedSourceCommit $sourceCommit `
            -OutputDirectory (Join-Path $fixtureRoot 'vhm-public-release-evidence-unsafe') `
            -PolicyPath $policyPath | Out-Null
    }
    catch {
        $unsafeRejected = $_.Exception.Message -match 'unsafe path segment'
    }
    Assert-True -Condition $unsafeRejected -Message 'Evidence archive path traversal was not rejected.'

    Write-Host 'public release preflight fixtures passed'
}
finally {
    $fixturePrefix = $fixtureParent + [System.IO.Path]::DirectorySeparatorChar + 'vhm-public-release-fixture-'
    $resolvedFixture = [System.IO.Path]::GetFullPath($fixtureRoot)
    if ([System.IO.Directory]::Exists($resolvedFixture) -and
        $resolvedFixture.StartsWith($fixturePrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        Remove-Item -LiteralPath $resolvedFixture -Recurse -Force
    }
}
