#requires -Version 7.2

[CmdletBinding()]
param(
    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string] $RepositoryRoot = (Join-Path $PSScriptRoot '..\..'),

    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string] $PolicyPath = (Join-Path $PSScriptRoot '..\..\release\public-release-policy.json'),

    [Parameter()]
    [string] $OutputPath,

    [Parameter()]
    [ValidatePattern('^[0-9A-Fa-f]{40}$')]
    [string] $ExpectedSourceCommit,

    [Parameter()]
    [switch] $RequireReady,

    [Parameter()]
    [switch] $ExpectBlocked
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($RequireReady -and $ExpectBlocked) {
    throw '-RequireReady and -ExpectBlocked are mutually exclusive.'
}

function Get-CanonicalPath {
    param(
        [Parameter(Mandatory)] [string] $LiteralPath,
        [Parameter(Mandatory)] [bool] $Directory,
        [Parameter(Mandatory)] [string] $Description
    )

    $resolved = @(Resolve-Path -LiteralPath $LiteralPath -ErrorAction Stop)
    if ($resolved.Count -ne 1) {
        throw "$Description must resolve to exactly one path."
    }
    $item = Get-Item -LiteralPath $resolved[0].ProviderPath -Force
    if ($Directory -ne [bool] $item.PSIsContainer) {
        throw "$Description has the wrong file type: '$($item.FullName)'."
    }
    if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$Description must not be a reparse point: '$($item.FullName)'."
    }
    return [System.IO.Path]::GetFullPath($item.FullName)
}

function Resolve-RepositoryFile {
    param(
        [Parameter(Mandatory)] [string] $Root,
        [Parameter(Mandatory)] [AllowNull()] [AllowEmptyString()] [string] $RelativePath,
        [Parameter(Mandatory)] [string] $Description,
        [switch] $Optional
    )

    if ([string]::IsNullOrWhiteSpace($RelativePath) -or [System.IO.Path]::IsPathRooted($RelativePath) -or
        $RelativePath.Contains(':') -or $RelativePath.IndexOf([char] 0) -ge 0) {
        if ($Optional) { return $null }
        throw "$Description must be a safe repository-relative path."
    }
    $segments = $RelativePath.Replace('\', '/').Split('/')
    if (@($segments | Where-Object { [string]::IsNullOrWhiteSpace($_) -or $_ -in @('.', '..') }).Count -ne 0) {
        throw "$Description contains an unsafe path segment."
    }
    $candidate = [System.IO.Path]::GetFullPath((Join-Path $Root ($segments -join [System.IO.Path]::DirectorySeparatorChar)))
    $prefix = $Root.TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
    if (-not $candidate.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "$Description escapes the repository root."
    }
    if (-not [System.IO.File]::Exists($candidate)) {
        if ($Optional) { return $null }
        throw "$Description does not exist: '$candidate'."
    }
    return Get-CanonicalPath -LiteralPath $candidate -Directory $false -Description $Description
}

function Add-Blocker {
    param(
        [Parameter(Mandatory)] [string] $Code,
        [Parameter(Mandatory)] [string] $Message,
        [string] $PolicyField
    )

    $blockers.Add([ordered]@{
            code = $Code
            policyField = $PolicyField
            message = $Message
        })
}

function Test-ApprovedReference {
    param([Parameter(Mandatory)] [object] $Section)
    $approvedProperty = $Section.PSObject.Properties['approved']
    $referenceProperty = $Section.PSObject.Properties['approvalReference']
    return $null -ne $approvedProperty -and $approvedProperty.Value -is [bool] -and
        $approvedProperty.Value -eq $true -and
        $null -ne $referenceProperty -and $referenceProperty.Value -is [string] -and
        -not [string]::IsNullOrWhiteSpace([string] $referenceProperty.Value)
}

function Test-BooleanTrue {
    param(
        [Parameter(Mandatory)] [object] $Section,
        [Parameter(Mandatory)] [string] $PropertyName
    )
    $property = $Section.PSObject.Properties[$PropertyName]
    return $null -ne $property -and $property.Value -is [bool] -and $property.Value -eq $true
}

function Get-RequiredEvidenceString {
    param(
        [Parameter(Mandatory)] [object] $Object,
        [Parameter(Mandatory)] [string] $PropertyName,
        [Parameter(Mandatory)] [string] $Context
    )
    $property = $Object.PSObject.Properties[$PropertyName]
    if ($null -eq $property -or $property.Value -isnot [string] -or [string]::IsNullOrWhiteSpace([string] $property.Value)) {
        throw "$Context requires a non-empty string '$PropertyName'."
    }
    return [string] $property.Value
}

function Get-RepositoryHeadCommit {
    param([Parameter(Mandatory)] [string] $Root)
    $git = Get-Command git -CommandType Application -ErrorAction Stop
    $output = & $git.Source -C $Root rev-parse --verify HEAD 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "Could not resolve repository HEAD: $($output -join ' ')"
    }
    $commit = ([string] ($output -join '')).Trim()
    if ($commit -cnotmatch '^[0-9a-f]{40}$') {
        throw 'Repository HEAD is not a full Git commit hash.'
    }
    return $commit
}

function Get-EvidenceValidationError {
    param(
        [Parameter(Mandatory)] [string] $Root,
        [Parameter(Mandatory)] [object] $PolicySection,
        [Parameter(Mandatory)] [string] $PolicyRequiredProperty,
        [Parameter(Mandatory)] [string] $EvidenceRecordsProperty,
        [Parameter(Mandatory)] [string[]] $RequiredCodes,
        [Parameter(Mandatory)] [string] $Description,
        [AllowEmptyString()] [string] $RequiredSourceCommit
    )

    try {
        $policySourceCommit = Get-RequiredEvidenceString -Object $PolicySection -PropertyName 'sourceCommit' -Context "$Description policy"
        if ($policySourceCommit -cnotmatch '^[0-9a-f]{40}$') {
            throw "$Description policy sourceCommit must be a full lowercase Git commit hash."
        }
        $releaseCommit = if ([string]::IsNullOrWhiteSpace($RequiredSourceCommit)) {
            Get-RepositoryHeadCommit -Root $Root
        }
        else {
            $RequiredSourceCommit.ToLowerInvariant()
        }
        if ($policySourceCommit -cne $releaseCommit) {
            throw "$Description policy sourceCommit does not match the release commit."
        }

        $policyCodes = @($PolicySection.$PolicyRequiredProperty)
        $policyCodeSet = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
        foreach ($code in $policyCodes) {
            if ($code -isnot [string] -or [string]::IsNullOrWhiteSpace([string] $code) -or -not $policyCodeSet.Add([string] $code)) {
                throw "$Description policy contains an invalid or duplicate required code."
            }
        }
        if ($policyCodeSet.Count -ne $RequiredCodes.Count -or
            @($RequiredCodes | Where-Object { -not $policyCodeSet.Contains($_) }).Count -ne 0) {
            throw "$Description policy does not contain the exact required scenario/check set."
        }

        $evidenceRelativePath = Get-RequiredEvidenceString -Object $PolicySection -PropertyName 'evidenceManifest' -Context "$Description policy"
        $evidencePath = Resolve-RepositoryFile -Root $Root -RelativePath $evidenceRelativePath -Description "$Description evidence manifest"
        $evidence = Get-Content -Raw -LiteralPath $evidencePath -Encoding UTF8 | ConvertFrom-Json -Depth 100
        if ($evidence.schemaVersion -isnot [long] -and $evidence.schemaVersion -isnot [int]) {
            throw "$Description evidence schemaVersion must be an integer."
        }
        if ([long] $evidence.schemaVersion -ne 1 -or [string] $evidence.sourceCommit -cne $releaseCommit) {
            throw "$Description evidence schema or sourceCommit is invalid."
        }

        $recordsProperty = $evidence.PSObject.Properties[$EvidenceRecordsProperty]
        if ($null -eq $recordsProperty) {
            throw "$Description evidence is missing '$EvidenceRecordsProperty'."
        }
        $records = @($recordsProperty.Value)
        $recordCodes = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
        foreach ($record in $records) {
            $code = Get-RequiredEvidenceString -Object $record -PropertyName 'code' -Context "$Description evidence record"
            if (-not $recordCodes.Add($code) -or $code -cnotin $RequiredCodes) {
                throw "$Description evidence contains an unknown or duplicate code '$code'."
            }
            if ([string] $record.status -cne 'passed') {
                throw "$Description evidence record '$code' is not passed."
            }
            $artifactRelativePath = Get-RequiredEvidenceString -Object $record -PropertyName 'evidencePath' -Context "$Description evidence record '$code'"
            $artifactHash = Get-RequiredEvidenceString -Object $record -PropertyName 'sha256' -Context "$Description evidence record '$code'"
            if ($artifactHash -cnotmatch '^[0-9a-f]{64}$') {
                throw "$Description evidence record '$code' has an invalid SHA-256."
            }
            $artifactPath = Resolve-RepositoryFile -Root $Root -RelativePath $artifactRelativePath -Description "$Description evidence artifact '$code'"
            $actualHash = (Get-FileHash -LiteralPath $artifactPath -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($actualHash -cne $artifactHash) {
                throw "$Description evidence artifact '$code' does not match its SHA-256."
            }
        }
        if ($recordCodes.Count -ne $RequiredCodes.Count -or
            @($RequiredCodes | Where-Object { -not $recordCodes.Contains($_) }).Count -ne 0) {
            throw "$Description evidence does not contain every required scenario/check."
        }
        return $null
    }
    catch {
        return $_.Exception.Message
    }
}

$root = Get-CanonicalPath -LiteralPath $RepositoryRoot -Directory $true -Description 'repository root'
$policyFile = Get-CanonicalPath -LiteralPath $PolicyPath -Directory $false -Description 'public release policy'
$rootPrefix = $root.TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
if (-not $policyFile.StartsWith($rootPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Public release policy must be stored inside the repository root: '$policyFile'."
}
$policy = Get-Content -Raw -LiteralPath $policyFile -Encoding UTF8 | ConvertFrom-Json -Depth 100
if ([long] $policy.schemaVersion -ne 1 -or [string] $policy.releaseMode -cne 'public') {
    throw 'Unsupported public release policy schema or releaseMode.'
}

$tauriConfigPath = Resolve-RepositoryFile -Root $root -RelativePath 'src-tauri/tauri.conf.json' -Description 'Tauri config'
$tauriConfig = Get-Content -Raw -LiteralPath $tauriConfigPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
$ffmpegManifestPath = Resolve-RepositoryFile -Root $root -RelativePath ([string] $policy.ffmpeg.manifest) -Description 'FFmpeg manifest'
$ffmpegManifest = Get-Content -Raw -LiteralPath $ffmpegManifestPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
$blockers = [System.Collections.Generic.List[object]]::new()

if (-not (Test-ApprovedReference -Section $policy.projectLicense) -or
    [string]::IsNullOrWhiteSpace([string] $policy.projectLicense.spdxExpression)) {
    Add-Blocker -Code 'PROJECT_LICENSE_APPROVAL_MISSING' -PolicyField 'projectLicense' -Message 'Project license choice and approval are incomplete.'
}
elseif ($null -eq (Resolve-RepositoryFile -Root $root -RelativePath ([string] $policy.projectLicense.file) -Description 'project license file' -Optional)) {
    Add-Blocker -Code 'PROJECT_LICENSE_FILE_MISSING' -PolicyField 'projectLicense.file' -Message 'Approved project license file is missing.'
}

if (-not (Test-ApprovedReference -Section $policy.eula)) {
    Add-Blocker -Code 'EULA_APPROVAL_MISSING' -PolicyField 'eula' -Message 'Public-distribution EULA approval is incomplete.'
}
elseif ($null -eq (Resolve-RepositoryFile -Root $root -RelativePath ([string] $policy.eula.file) -Description 'EULA file' -Optional)) {
    Add-Blocker -Code 'EULA_FILE_MISSING' -PolicyField 'eula.file' -Message 'Approved EULA file is missing.'
}

$compliance = $policy.thirdPartyCompliance
$complianceRoot = [string] $compliance.root
foreach ($field in @('manifest', 'summary', 'notices', 'licenseTexts', 'npmRuntimeSbom', 'cargoWindowsX64Sbom')) {
    $relative = if ([string]::IsNullOrWhiteSpace($complianceRoot)) { $null } else { "$complianceRoot/$([string] $compliance.$field)" }
    if ($null -eq (Resolve-RepositoryFile -Root $root -RelativePath $relative -Description "third-party compliance $field" -Optional)) {
        Add-Blocker -Code 'THIRD_PARTY_EVIDENCE_MISSING' -PolicyField "thirdPartyCompliance.$field" -Message "Third-party compliance file '$field' is missing."
    }
}
if (-not (Test-ApprovedReference -Section $compliance)) {
    Add-Blocker -Code 'THIRD_PARTY_APPROVAL_MISSING' -PolicyField 'thirdPartyCompliance' -Message 'SBOM, notices, and license-text review are not approved.'
}
else {
    $summaryRelative = "$complianceRoot/$([string] $compliance.summary)"
    $summaryPath = Resolve-RepositoryFile -Root $root -RelativePath $summaryRelative -Description 'third-party compliance summary' -Optional
    if ($null -ne $summaryPath) {
        $summary = Get-Content -Raw -LiteralPath $summaryPath -Encoding UTF8 | ConvertFrom-Json -Depth 100
        if (-not (Test-BooleanTrue -Section $summary -PropertyName 'publicRedistributionReady')) {
            Add-Blocker -Code 'THIRD_PARTY_SUMMARY_BLOCKED' -PolicyField 'thirdPartyCompliance.summary' -Message 'Compliance summary still contains technical blockers.'
        }
    }
}

if (-not (Test-BooleanTrue -Section $policy.identity -PropertyName 'brandApproved') -or [string]::IsNullOrWhiteSpace([string] $policy.identity.brandApprovalReference)) {
    Add-Blocker -Code 'BRAND_APPROVAL_MISSING' -PolicyField 'identity.brandApproved' -Message 'Product-name and trademark review are incomplete.'
}
if ([string] $policy.identity.productName -cne [string] $tauriConfig.productName) {
    Add-Blocker -Code 'PRODUCT_NAME_MISMATCH' -PolicyField 'identity.productName' -Message 'Approved product name does not match tauri.conf.json.'
}
if (-not (Test-BooleanTrue -Section $policy.identity -PropertyName 'publisherApproved') -or
    [string]::IsNullOrWhiteSpace([string] $policy.identity.publisherSubject) -or
    [string]::IsNullOrWhiteSpace([string] $policy.identity.publisherApprovalReference)) {
    Add-Blocker -Code 'PUBLISHER_APPROVAL_MISSING' -PolicyField 'identity.publisherSubject' -Message 'Legal publisher and certificate subject are not approved.'
}
if (-not (Test-BooleanTrue -Section $policy.identity -PropertyName 'identifierApproved') -or
    [string]::IsNullOrWhiteSpace([string] $policy.identity.identifierApprovalReference)) {
    Add-Blocker -Code 'IDENTIFIER_APPROVAL_MISSING' -PolicyField 'identity.identifier' -Message 'Stable application identifier is not approved.'
}
if ([string] $policy.identity.identifier -cne [string] $tauriConfig.identifier) {
    Add-Blocker -Code 'IDENTIFIER_MISMATCH' -PolicyField 'identity.identifier' -Message 'Approved identifier does not match tauri.conf.json.'
}

$confirmedScopes = @($policy.iconRights.confirmedScopes)
$missingIconScopes = @($policy.iconRights.requiredScopes | Where-Object { $_ -cnotin $confirmedScopes })
if (-not (Test-ApprovedReference -Section $policy.iconRights) -or $missingIconScopes.Count -ne 0) {
    Add-Blocker -Code 'ICON_DISTRIBUTION_RIGHTS_MISSING' -PolicyField 'iconRights' -Message "Icon rights do not cover: $($missingIconScopes -join ', ')."
}
if (-not (Test-ApprovedReference -Section $policy.riotTencentDisclaimer) -or
    $null -eq (Resolve-RepositoryFile -Root $root -RelativePath ([string] $policy.riotTencentDisclaimer.materialPath) -Description 'Riot/Tencent disclaimer material' -Optional)) {
    Add-Blocker -Code 'DISCLAIMER_APPROVAL_MISSING' -PolicyField 'riotTencentDisclaimer' -Message 'Required non-affiliation/trademark disclaimer is not approved and present.'
}

$ffmpegSource = $ffmpegManifest.sourceCompliance
if (-not (Test-BooleanTrue -Section $ffmpegSource -PropertyName 'redistributionReady') -or [string] $ffmpegSource.status -cne 'ready-for-redistribution') {
    Add-Blocker -Code 'FFMPEG_REDISTRIBUTION_BLOCKED' -PolicyField 'ffmpeg.manifest' -Message "FFmpeg redistribution status is '$($ffmpegSource.status)'."
}

$signing = $policy.authenticode
if (-not (Test-BooleanTrue -Section $signing -PropertyName 'certificateProvisioned') -or
    [string]::IsNullOrWhiteSpace([string] $signing.expectedPublisherSubject) -or
    [string] $signing.expectedCertificateThumbprint -notmatch '^[0-9A-Fa-f]{40}$' -or
    [string] $signing.expectedPublisherSubject -cne [string] $policy.identity.publisherSubject -or
    [string]::IsNullOrWhiteSpace([string] $signing.approvalReference)) {
    Add-Blocker -Code 'CODE_SIGNING_NOT_READY' -PolicyField 'authenticode' -Message 'Authenticode certificate and approved publisher binding are incomplete.'
}
if (-not (Test-BooleanTrue -Section $signing -PropertyName 'trustedTimestampRequired') -or [string] $signing.timestampUrl -notmatch '^https://') {
    Add-Blocker -Code 'TIMESTAMP_NOT_READY' -PolicyField 'authenticode.timestampUrl' -Message 'Trusted HTTPS timestamp service is not configured.'
}
if (-not (Test-BooleanTrue -Section $signing -PropertyName 'signtoolVerificationRequired')) {
    Add-Blocker -Code 'SIGNTOOL_VERIFICATION_NOT_READY' -PolicyField 'authenticode.signtoolVerificationRequired' -Message 'Public artifacts must require signtool chain verification.'
}

if (-not (Test-ApprovedReference -Section $policy.cleanVmValidation)) {
    Add-Blocker -Code 'CLEAN_VM_EVIDENCE_MISSING' -PolicyField 'cleanVmValidation' -Message 'Approved clean-VM install/upgrade/uninstall evidence is missing.'
}
else {
    $requiredCleanVmScenarios = @(
        'fresh-install-webview2-present',
        'fresh-install-webview2-missing-online',
        'fresh-install-webview2-missing-offline',
        'same-version-reinstall',
        'supported-upgrade',
        'downgrade-rejected',
        'uninstall-user-data-preserved',
        'packaged-ffmpeg-only'
    )
    $cleanVmError = Get-EvidenceValidationError `
        -Root $root `
        -PolicySection $policy.cleanVmValidation `
        -PolicyRequiredProperty 'requiredScenarios' `
        -EvidenceRecordsProperty 'scenarios' `
        -RequiredCodes $requiredCleanVmScenarios `
        -Description 'clean VM' `
        -RequiredSourceCommit $ExpectedSourceCommit
    if (-not [string]::IsNullOrWhiteSpace($cleanVmError)) {
        Add-Blocker -Code 'CLEAN_VM_EVIDENCE_INVALID' -PolicyField 'cleanVmValidation.evidenceManifest' -Message $cleanVmError
    }
}

$updaterDecision = [string] $policy.updater.decision
if ($updaterDecision -cnotin @('enabled', 'disabled') -or [string]::IsNullOrWhiteSpace([string] $policy.updater.approvalReference)) {
    Add-Blocker -Code 'UPDATER_DECISION_PENDING' -PolicyField 'updater.decision' -Message 'Updater enable/disable decision is not approved.'
}
elseif ($updaterDecision -ceq 'enabled' -and
    ([string] $policy.updater.endpoint -notmatch '^https://' -or [string]::IsNullOrWhiteSpace([string] $policy.updater.publicKeyReference))) {
    Add-Blocker -Code 'UPDATER_CONFIGURATION_INCOMPLETE' -PolicyField 'updater' -Message 'Enabled updater lacks an HTTPS endpoint or public-key reference.'
}

if (-not (Test-ApprovedReference -Section $policy.dataSafety)) {
    Add-Blocker -Code 'DATA_SAFETY_APPROVAL_MISSING' -PolicyField 'dataSafety' -Message 'Source-media and user-data safety evidence is not approved.'
}
else {
    $requiredDataSafetyChecks = @(
        'source-media-readonly-default',
        'permanent-delete-explicit-confirmation',
        'application-data-boundary',
        'uninstall-user-data-preserved'
    )
    $dataSafetyError = Get-EvidenceValidationError `
        -Root $root `
        -PolicySection $policy.dataSafety `
        -PolicyRequiredProperty 'requiredChecks' `
        -EvidenceRecordsProperty 'checks' `
        -RequiredCodes $requiredDataSafetyChecks `
        -Description 'data safety' `
        -RequiredSourceCommit $ExpectedSourceCommit
    if (-not [string]::IsNullOrWhiteSpace($dataSafetyError)) {
        Add-Blocker -Code 'DATA_SAFETY_EVIDENCE_INVALID' -PolicyField 'dataSafety.evidenceManifest' -Message $dataSafetyError
    }
}

$report = [ordered]@{
    schemaVersion = 1
    status = if ($blockers.Count -eq 0) { 'passed' } else { 'blocked' }
    checkedAtUtc = [DateTime]::UtcNow.ToString('o')
    releaseMode = 'public'
    policy = [ordered]@{
        path = $policyFile
        sha256 = (Get-FileHash -LiteralPath $policyFile -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    product = [ordered]@{
        name = [string] $tauriConfig.productName
        version = [string] $tauriConfig.version
        identifier = [string] $tauriConfig.identifier
    }
    blockerCount = $blockers.Count
    blockers = $blockers.ToArray()
}
$json = $report | ConvertTo-Json -Depth 12

if (-not [string]::IsNullOrWhiteSpace($OutputPath)) {
    $outputFull = [System.IO.Path]::GetFullPath($OutputPath)
    if ([System.IO.File]::Exists($outputFull)) {
        throw "Preflight output already exists: '$outputFull'."
    }
    $parent = [System.IO.Directory]::GetParent($outputFull)
    if ($null -eq $parent -or -not $parent.Exists) {
        throw "Preflight output parent does not exist: '$outputFull'."
    }
    $json | Set-Content -LiteralPath $outputFull -Encoding UTF8
}

$json
if ($RequireReady -and $blockers.Count -ne 0) {
    throw "Public release preflight is blocked by $($blockers.Count) issue(s)."
}
if ($ExpectBlocked -and $blockers.Count -eq 0) {
    throw 'Public release preflight unexpectedly passed.'
}
