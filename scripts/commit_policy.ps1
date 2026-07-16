Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-RepoRoot {
    return (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
}

function Get-CommitPolicy {
    $policyPath = Join-Path $PSScriptRoot "commit_rules.json"
    if (-not (Test-Path -LiteralPath $policyPath)) {
        throw "Commit policy file not found: $policyPath"
    }
    $policy = Get-Content -LiteralPath $policyPath -Raw | ConvertFrom-Json -AsHashtable
    foreach ($requiredKey in @("all_types", "releasable_types", "src_prefixes")) {
        if (-not $policy.ContainsKey($requiredKey)) {
            throw "Commit policy is missing '$requiredKey'."
        }
    }
    return $policy
}

function Test-HasSrcChanges {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [string[]]$Paths,
        [Parameter(Mandatory = $true)]
        [hashtable]$Policy
    )

    foreach ($path in $Paths) {
        foreach ($prefix in @($Policy["src_prefixes"])) {
            if ($path -like "$prefix*") {
                return $true
            }
        }
    }
    return $false
}

function Parse-ChangeEntryLine {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$Line,
        [Parameter(Mandatory = $true)]
        [hashtable]$Policy
    )

    $trimmed = $Line.Trim()
    if ([string]::IsNullOrWhiteSpace($trimmed)) {
        return $null
    }
    $match = [regex]::Match($trimmed, '^(?<type>[a-z]+)(?<breaking>!)?: (?<description>.+)$')
    if (-not $match.Success) {
        return $null
    }

    $type = $match.Groups["type"].Value
    if ($type -notin @($Policy["all_types"])) {
        return $null
    }

    return @{
        Raw         = $trimmed
        Type        = $type
        IsBreaking  = $match.Groups["breaking"].Success
        Description = $match.Groups["description"].Value
    }
}

function Split-CommitMessage {
    param(
        [Parameter(Mandatory = $true)]
        [string]$MessageText,
        [Parameter(Mandatory = $true)]
        [hashtable]$Policy
    )

    $normalized = $MessageText -replace "`r", ""
    $lines = @($normalized -split "`n")
    if ($lines.Count -eq 0 -or [string]::IsNullOrWhiteSpace($lines[0])) {
        throw "Commit message subject line is empty."
    }

    $subjectEntry = Parse-ChangeEntryLine -Line $lines[0] -Policy $Policy
    if ($null -eq $subjectEntry) {
        throw "Commit subject must follow '<type>!: <description>' (or '<type>: <description>') and use an allowed type."
    }

    $bodyLines = @()
    if ($lines.Count -gt 1) {
        $bodyLines = @($lines[1..($lines.Count - 1)])
    }

    $end = $bodyLines.Count - 1
    while ($end -ge 0 -and [string]::IsNullOrWhiteSpace($bodyLines[$end])) {
        $end--
    }
    if ($end -lt 0) {
        return @{
            SubjectEntry = $subjectEntry
            FooterEntries = @()
            FreeformLines = @()
        }
    }

    $footerEntries = New-Object System.Collections.Generic.List[hashtable]
    $cursor = $end
    while ($cursor -ge 0) {
        $candidate = Parse-ChangeEntryLine -Line $bodyLines[$cursor] -Policy $Policy
        if ($null -eq $candidate) {
            break
        }
        $footerEntries.Add($candidate)
        $cursor--
    }

    $footerArray = @($footerEntries.ToArray())
    [array]::Reverse($footerArray)
    $freeform = @()
    if ($cursor -ge 0) {
        $freeform = @($bodyLines[0..$cursor])
    }

    return @{
        SubjectEntry = $subjectEntry
        FooterEntries = $footerArray
        FreeformLines = $freeform
    }
}

function Validate-CommitMessageAgainstPolicy {
    param(
        [Parameter(Mandatory = $true)]
        [string]$MessageText,
        [Parameter(Mandatory = $true)]
        [bool]$HasSrcChanges,
        [Parameter(Mandatory = $true)]
        [hashtable]$Policy
    )

    $sections = Split-CommitMessage -MessageText $MessageText -Policy $Policy
    $entries = @($sections.SubjectEntry) + @($sections.FooterEntries)
    $releasable = @($Policy["releasable_types"])

    foreach ($entry in $entries) {
        $isReleasable = $entry.Type -in $releasable
        if ($entry.IsBreaking -and -not $isReleasable) {
            throw "Only releasable types ($($releasable -join ', ')) may use the breaking marker (!): '$($entry.Raw)'"
        }
        if (-not $HasSrcChanges -and $isReleasable) {
            throw "Type '$($entry.Type)' is reserved for commits that include src changes."
        }
    }

    return $sections
}
