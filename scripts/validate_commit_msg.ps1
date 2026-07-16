#!/usr/bin/env pwsh
param(
    [Parameter(Mandatory = $true)]
    [string]$CommitMessageFile
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot "commit_policy.ps1")

if (-not (Test-Path -LiteralPath $CommitMessageFile)) {
    throw "Commit message file not found: $CommitMessageFile"
}

$policy = Get-CommitPolicy
$messageText = Get-Content -LiteralPath $CommitMessageFile -Raw

$stagedPathsRaw = @(& git diff --cached --name-only)
$stagedPaths = @()
foreach ($line in $stagedPathsRaw) {
    $trimmed = "$line".Trim()
    if (-not [string]::IsNullOrWhiteSpace($trimmed)) {
        $stagedPaths += $trimmed
    }
}
$hasSrcChanges = Test-HasSrcChanges -Paths $stagedPaths -Policy $policy

try {
    $null = Validate-CommitMessageAgainstPolicy -MessageText $messageText -HasSrcChanges:$hasSrcChanges -Policy $policy
} catch {
    Write-Error $_
    exit 1
}

exit 0
