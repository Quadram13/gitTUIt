#!/usr/bin/env pwsh
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot "commit_policy.ps1")

function Ensure-Command([string]$Name) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Required command '$Name' is not available."
    }
}

function Prompt-YesNo([string]$Question, [bool]$DefaultNo = $true) {
    $hint = if ($DefaultNo) { "[y/N]" } else { "[Y/n]" }
    while ($true) {
        $answer = (Read-Host "$Question $hint").Trim().ToLowerInvariant()
        if ([string]::IsNullOrWhiteSpace($answer)) {
            return -not $DefaultNo
        }
        if ($answer -in @("y", "yes")) {
            return $true
        }
        if ($answer -in @("n", "no")) {
            return $false
        }
        Write-Host "Please answer y or n." -ForegroundColor Yellow
    }
}

function Prompt-MenuSelection([string]$Title, [string[]]$Options) {
    if ($Options.Count -eq 0) {
        throw "No options available for selection."
    }
    Write-Host $Title
    for ($i = 0; $i -lt $Options.Count; $i++) {
        Write-Host "  $($i + 1). $($Options[$i])"
    }
    while ($true) {
        $raw = Read-Host "Choose option (1-$($Options.Count))"
        $choice = 0
        if ([int]::TryParse($raw, [ref]$choice) -and $choice -ge 1 -and $choice -le $Options.Count) {
            return $Options[$choice - 1]
        }
        Write-Host "Invalid selection." -ForegroundColor Yellow
    }
}

function Read-MultilineInput([string]$PromptText) {
    Write-Host $PromptText
    Write-Host "Submit by entering an empty line."
    $lines = New-Object System.Collections.Generic.List[string]
    while ($true) {
        $line = Read-Host
        if ($line -eq "") {
            break
        }
        $lines.Add($line) | Out-Null
    }
    return @($lines.ToArray())
}

function Get-CurrentBranch {
    Ensure-Command "git"
    return (& git rev-parse --abbrev-ref HEAD).Trim()
}

function Get-WorkingTreeChanges {
    Ensure-Command "git"
    $raw = @(& git status --porcelain)
    $changes = @()
    foreach ($line in $raw) {
        $trimmed = "$line".Trim()
        if (-not [string]::IsNullOrWhiteSpace($trimmed)) {
            $changes += $trimmed
        }
    }
    return $changes
}

function Get-StagedPaths {
    Ensure-Command "git"
    $raw = @(& git diff --cached --name-only)
    $paths = @()
    foreach ($line in $raw) {
        $trimmed = "$line".Trim()
        if (-not [string]::IsNullOrWhiteSpace($trimmed)) {
            $paths += $trimmed
        }
    }
    return $paths
}

function Ensure-CleanWorkingTree {
    $changes = @(Get-WorkingTreeChanges)
    if ($changes.Count -gt 0) {
        throw "Working tree has uncommitted changes. Commit/stash before running this task."
    }
}

function Ensure-BranchUpstream([string]$Branch) {
    Ensure-Command "git"
    try {
        $null = & git rev-parse --abbrev-ref --symbolic-full-name "@{u}" 2>$null
    } catch {
        Write-Host "No upstream configured for '$Branch'. Pushing with upstream tracking..." -ForegroundColor Yellow
        & git push -u origin $Branch
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to push '$Branch' and configure upstream."
        }
    }
}

function Prompt-CommitEntry([hashtable]$Policy, [bool]$HasSrcChanges) {
    $allTypes = @($Policy["all_types"])
    $releasable = @($Policy["releasable_types"])
    $allowedTypes = if ($HasSrcChanges) {
        $allTypes
    } else {
        @($allTypes | Where-Object { $_ -notin $releasable })
    }
    $selectedType = Prompt-MenuSelection -Title "Select commit change type" -Options $allowedTypes

    $description = ""
    while ([string]::IsNullOrWhiteSpace($description)) {
        $description = (Read-Host "Enter change description").Trim()
    }

    $isBreaking = $false
    if ($selectedType -in $releasable) {
        $isBreaking = Prompt-YesNo -Question "Mark this entry as breaking?" -DefaultNo $true
    }

    $breakingMark = if ($isBreaking) { "!" } else { "" }
    return "$selectedType$breakingMark`: $description"
}

function Build-CommitMessageText([string[]]$Entries, [string[]]$FreeformLines) {
    if ($Entries.Count -eq 0) {
        throw "At least one change entry is required."
    }
    $subject = $Entries[0]
    $otherEntries = @()
    if ($Entries.Count -gt 1) {
        $otherEntries = @($Entries[1..($Entries.Count - 1)])
    }

    $messageLines = New-Object System.Collections.Generic.List[string]
    $messageLines.Add($subject) | Out-Null

    if ($FreeformLines.Count -gt 0 -or $otherEntries.Count -gt 0) {
        $messageLines.Add("") | Out-Null
    }
    foreach ($line in $FreeformLines) {
        $messageLines.Add($line) | Out-Null
    }
    if ($FreeformLines.Count -gt 0 -and $otherEntries.Count -gt 0) {
        $messageLines.Add("") | Out-Null
    }
    foreach ($entry in $otherEntries) {
        $messageLines.Add($entry) | Out-Null
    }

    return (@($messageLines.ToArray()) -join "`n")
}

function Commit-Workflow {
    Ensure-Command "git"
    $policy = Get-CommitPolicy

    & git add -A
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to stage changes."
    }

    $stagedPaths = @(Get-StagedPaths)
    if ($stagedPaths.Count -eq 0) {
        Write-Host "No staged changes found. Nothing to commit." -ForegroundColor Yellow
        return
    }
    $hasSrcChanges = Test-HasSrcChanges -Paths $stagedPaths -Policy $policy
    Write-Host ("Detected src changes: {0}" -f $hasSrcChanges) -ForegroundColor Cyan

    $entries = New-Object System.Collections.Generic.List[string]
    $entries.Add((Prompt-CommitEntry -Policy $policy -HasSrcChanges:$hasSrcChanges)) | Out-Null

    if (Prompt-YesNo -Question "Add additional change entries?" -DefaultNo $true) {
        while ($true) {
            $entries.Add((Prompt-CommitEntry -Policy $policy -HasSrcChanges:$hasSrcChanges)) | Out-Null
            if (-not (Prompt-YesNo -Question "Add another entry?" -DefaultNo $true)) {
                break
            }
        }
    }

    $freeform = @()
    if (Prompt-YesNo -Question "Add freeform body paragraphs?" -DefaultNo $true) {
        $freeform = @(Read-MultilineInput -PromptText "Enter freeform body lines")
    }

    $messageText = Build-CommitMessageText -Entries @($entries.ToArray()) -FreeformLines $freeform
    $null = Validate-CommitMessageAgainstPolicy -MessageText $messageText -HasSrcChanges:$hasSrcChanges -Policy $policy

    $tempFile = [System.IO.Path]::GetTempFileName()
    try {
        Set-Content -LiteralPath $tempFile -Value $messageText -NoNewline
        & git commit -F $tempFile
        if ($LASTEXITCODE -ne 0) {
            throw "git commit failed."
        }
    } finally {
        if (Test-Path -LiteralPath $tempFile) {
            Remove-Item -LiteralPath $tempFile -Force
        }
    }
}

function Push-Workflow {
    Ensure-Command "git"
    $branch = Get-CurrentBranch
    if ($branch -eq "HEAD") {
        throw "Detached HEAD state detected. Checkout a branch first."
    }

    $changes = @(Get-WorkingTreeChanges)
    if ($changes.Count -gt 0) {
        if (Prompt-YesNo -Question "Uncommitted changes found. Run commit task before push?" -DefaultNo $true) {
            Commit-Workflow
        } else {
            Write-Host "Leaving uncommitted changes untouched; pushing existing commits only." -ForegroundColor Yellow
        }
    }

    try {
        $null = & git rev-parse --abbrev-ref --symbolic-full-name "@{u}" 2>$null
        & git push
    } catch {
        & git push -u origin $branch
    }
    if ($LASTEXITCODE -ne 0) {
        throw "Push failed."
    }
}

function Get-CommitRecordsSinceBase([string]$Branch, [string]$BaseBranch, [hashtable]$Policy) {
    Ensure-Command "git"
    $baseRef = "origin/$BaseBranch"
    try {
        & git rev-parse --verify $baseRef *> $null
    } catch {
        $baseRef = $BaseBranch
    }

    $mergeBase = (& git merge-base $Branch $baseRef).Trim()
    if ([string]::IsNullOrWhiteSpace($mergeBase)) {
        throw "Could not determine merge-base for '$Branch' and '$baseRef'."
    }

    $range = "$mergeBase..$Branch"
    $raw = & git log --reverse --format="%H%x1f%s%x1f%b%x1e" $range
    $text = ($raw -join "`n")
    $records = @($text -split [char]0x1e)
    $commits = New-Object System.Collections.Generic.List[hashtable]

    foreach ($record in $records) {
        $entry = "$record".Trim()
        if ([string]::IsNullOrWhiteSpace($entry)) {
            continue
        }
        $parts = @($entry -split [char]0x1f, 3)
        if ($parts.Count -lt 2) {
            continue
        }
        $hash = $parts[0].Trim()
        $subject = $parts[1].Trim()
        $body = if ($parts.Count -gt 2) { $parts[2] -replace "`r", "" } else { "" }
        if ([string]::IsNullOrWhiteSpace($subject)) {
            continue
        }

        $message = if ([string]::IsNullOrWhiteSpace($body)) { $subject } else { "$subject`n$body" }
        $sections = Split-CommitMessage -MessageText $message -Policy $Policy
        $entryList = @($sections.SubjectEntry) + @($sections.FooterEntries)
        $commits.Add(@{
                Hash = $hash
                Subject = $subject
                Entries = $entryList
                FreeformLines = @($sections.FreeformLines)
            }) | Out-Null
    }
    return @($commits.ToArray())
}

function Prompt-SelectTitleEntry($Commits) {
    $flattened = New-Object System.Collections.Generic.List[hashtable]
    foreach ($commit in $Commits) {
        $short = $commit.Hash.Substring(0, [Math]::Min(7, $commit.Hash.Length))
        foreach ($entry in @($commit.Entries)) {
            $flattened.Add(@{
                    Hash = $commit.Hash
                    Display = "[$short] $($entry.Raw)"
                    Raw = $entry.Raw
                }) | Out-Null
        }
    }

    if ($flattened.Count -eq 0) {
        throw "No change entries were discovered from branch commits."
    }

    $options = @()
    foreach ($item in @($flattened.ToArray())) {
        $options += $item.Display
    }
    $selected = Prompt-MenuSelection -Title "Select PR title change entry" -Options $options
    foreach ($item in @($flattened.ToArray())) {
        if ($item.Display -eq $selected) {
            return $item
        }
    }
    throw "Could not resolve selected title entry."
}

function Build-PullRequestBody($Commits, [hashtable]$SelectedTitle, [bool]$IncludeFreeform) {
    $remainingEntries = New-Object System.Collections.Generic.List[string]
    foreach ($commit in $Commits) {
        foreach ($entry in @($commit.Entries)) {
            $raw = $entry.Raw
            if ($raw -eq $SelectedTitle.Raw -and $commit.Hash -eq $SelectedTitle.Hash) {
                continue
            }
            $remainingEntries.Add($raw) | Out-Null
        }
    }

    $lines = New-Object System.Collections.Generic.List[string]
    $lines.Add("## Change Entries") | Out-Null
    if ($remainingEntries.Count -eq 0) {
        $lines.Add("- (none)") | Out-Null
    } else {
        foreach ($entry in @($remainingEntries.ToArray())) {
            $lines.Add("- $entry") | Out-Null
        }
    }

    if ($IncludeFreeform) {
        $lines.Add("") | Out-Null
        $lines.Add("## Additional Context") | Out-Null
        $foundAny = $false
        foreach ($commit in $Commits) {
            $content = @($commit.FreeformLines | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
            if ($content.Count -eq 0) {
                continue
            }
            $short = $commit.Hash.Substring(0, [Math]::Min(7, $commit.Hash.Length))
            $lines.Add("### $short $($commit.Subject)") | Out-Null
            foreach ($line in $content) {
                $lines.Add($line) | Out-Null
            }
            $lines.Add("") | Out-Null
            $foundAny = $true
        }
        if (-not $foundAny) {
            $lines.Add("(No freeform body paragraphs found in commit history.)") | Out-Null
        }
    }
    return (@($lines.ToArray()) -join "`n").Trim()
}

function PullRequest-Workflow([string]$BaseBranch) {
    Ensure-Command "git"
    Ensure-Command "gh"
    $policy = Get-CommitPolicy

    $branch = Get-CurrentBranch
    if ($branch -eq "HEAD") {
        throw "Detached HEAD state detected. Checkout a branch first."
    }
    if ($branch -eq $BaseBranch -or $branch -eq "main") {
        throw "Refusing PR creation from '$branch'. Use a non-main branch."
    }

    Ensure-BranchUpstream -Branch $branch
    Ensure-CleanWorkingTree

    $commits = @(Get-CommitRecordsSinceBase -Branch $branch -BaseBranch $BaseBranch -Policy $policy)
    if ($commits.Count -eq 0) {
        throw "No commits found between '$branch' and '$BaseBranch'."
    }
    $selectedTitle = Prompt-SelectTitleEntry -Commits $commits
    $includeFreeform = Prompt-YesNo -Question "Include freeform body paragraphs from commits in PR body?" -DefaultNo $true
    $body = Build-PullRequestBody -Commits $commits -SelectedTitle $selectedTitle -IncludeFreeform:$includeFreeform

    & gh pr create --base $BaseBranch --head $branch --title $selectedTitle.Raw --body $body
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to create pull request via GitHub CLI."
    }
}

function Assert-PrReadyForMerge([string]$Branch) {
    Ensure-Command "gh"
    $raw = & gh pr view $Branch --json number,title,url,state,mergeStateStatus,statusCheckRollup
    if ($LASTEXITCODE -ne 0) {
        throw "Could not inspect pull request for branch '$Branch'."
    }
    $pr = $raw | ConvertFrom-Json
    if ("$($pr.state)" -ne "OPEN") {
        throw "PR for '$Branch' is not open."
    }

    $mergeState = "$($pr.mergeStateStatus)".ToUpperInvariant()
    if ($mergeState -in @("BLOCKED", "DIRTY", "DRAFT", "UNKNOWN", "BEHIND")) {
        throw "PR #$($pr.number) is not merge-ready (mergeStateStatus=$mergeState). Resolve PR readiness checks and retry."
    }

    $failed = New-Object System.Collections.Generic.List[string]
    $pending = New-Object System.Collections.Generic.List[string]
    foreach ($item in @($pr.statusCheckRollup)) {
        $name = "$($item.name)"
        if ([string]::IsNullOrWhiteSpace($name)) {
            $name = "$($item.context)"
        }
        if ([string]::IsNullOrWhiteSpace($name)) {
            $name = "unnamed-check"
        }
        $state = "$($item.conclusion)"
        if ([string]::IsNullOrWhiteSpace($state)) {
            $state = "$($item.status)"
        }
        if ([string]::IsNullOrWhiteSpace($state)) {
            $state = "$($item.state)"
        }
        $normalized = $state.ToUpperInvariant()
        if ($normalized -in @("SUCCESS", "NEUTRAL", "SKIPPED", "EXPECTED")) {
            continue
        }
        if ($normalized -in @("PENDING", "QUEUED", "IN_PROGRESS", "WAITING", "REQUESTED")) {
            $pending.Add($name) | Out-Null
        } else {
            $failed.Add("$name ($normalized)") | Out-Null
        }
    }

    if ($failed.Count -gt 0 -or $pending.Count -gt 0) {
        $messages = New-Object System.Collections.Generic.List[string]
        if ($failed.Count -gt 0) {
            $messages.Add("failing checks: $($failed -join ', ')") | Out-Null
        }
        if ($pending.Count -gt 0) {
            $messages.Add("pending checks: $($pending -join ', ')") | Out-Null
        }
        throw "PR #$($pr.number) is not ready to merge ($($messages -join '; ')). This task fails one-shot; rerun after checks pass."
    }
}

function Get-PullRequestMetadata([string]$Branch) {
    Ensure-Command "gh"
    $raw = & gh pr view $Branch --json number,title,url,state,mergeStateStatus,statusCheckRollup
    if ($LASTEXITCODE -ne 0) {
        throw "Could not inspect pull request for branch '$Branch'."
    }
    return ($raw | ConvertFrom-Json)
}

function Test-IsReleasePullRequest($Pr) {
    $title = "$($Pr.title)"
    return $title -match '^chore:\s+release\b'
}

function Merge-PullRequestFlow([string]$Branch, [bool]$ExpectReleasePr = $false) {
    Ensure-Command "git"
    Ensure-Command "gh"
    if ([string]::IsNullOrWhiteSpace($Branch)) {
        $Branch = Get-CurrentBranch
    }
    if ($Branch -eq "HEAD") {
        throw "Detached HEAD state detected. Checkout a branch first."
    }

    $pr = Get-PullRequestMetadata -Branch $Branch
    $isReleasePr = Test-IsReleasePullRequest -Pr $pr
    if ($ExpectReleasePr -and -not $isReleasePr) {
        throw "Branch '$Branch' does not point to a release PR. Use merge-pr for non-release PRs."
    }
    if (-not $ExpectReleasePr -and $isReleasePr) {
        throw "Branch '$Branch' points to a release PR. Use merge-release-pr instead."
    }

    Assert-PrReadyForMerge -Branch $Branch
    $mergeArgs = @($Branch, "--merge")
    if (Prompt-YesNo -Question "Delete branch after merge?" -DefaultNo $true) {
        $mergeArgs += "--delete-branch"
    }

    & gh pr merge @mergeArgs
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to merge pull request."
    }
}

function Show-ReleaseStatus([string]$BaseBranch) {
    Ensure-Command "git"
    Write-Host "Release workflow status" -ForegroundColor Cyan
    Write-Host "-----------------------"
    Write-Host "Current branch: $(Get-CurrentBranch)"
    $tagRaw = @(& git describe --tags --abbrev=0 2>$null)
    $latestTag = ""
    if ($tagRaw.Count -gt 0) {
        $latestTag = "$($tagRaw[0])".Trim()
    }
    if ([string]::IsNullOrWhiteSpace($latestTag)) {
        $latestTag = "(no tags yet)"
    }
    Write-Host "Latest tag: $latestTag"
    Write-Host ""
    Write-Host "Working tree:"
    & git status --short

    if (Get-Command "gh" -ErrorAction SilentlyContinue) {
        $json = & gh pr list --state open --base $BaseBranch --search "chore: release" --limit 20 --json number,title,url
        if ($LASTEXITCODE -eq 0) {
            $prs = @($json | ConvertFrom-Json)
            Write-Host ""
            if ($prs.Count -eq 0) {
                Write-Host "Open release PRs: none"
            } else {
                Write-Host "Open release PRs:"
                foreach ($pr in $prs) {
                    Write-Host "  #$($pr.number) $($pr.title)"
                    Write-Host "    $($pr.url)"
                }
            }
        }
    }
}

if ($args.Count -eq 0) {
    throw "Usage: workspace_release.ps1 <commit|push|pr|merge-pr|merge-release-pr|status> [args]"
}

$command = $args[0].Trim().ToLowerInvariant()
switch ($command) {
    "commit" {
        Commit-Workflow
    }
    "push" {
        Push-Workflow
    }
    "pr" {
        $base = "main"
        if ($args.Count -ge 2) {
            $base = $args[1]
        }
        PullRequest-Workflow -BaseBranch $base
    }
    "merge-pr" {
        $branch = ""
        if ($args.Count -ge 2) {
            $branch = $args[1].Trim()
        }
        Merge-PullRequestFlow -Branch $branch -ExpectReleasePr:$false
    }
    "merge-release-pr" {
        $branch = ""
        if ($args.Count -ge 2) {
            $branch = $args[1].Trim()
        }
        Merge-PullRequestFlow -Branch $branch -ExpectReleasePr:$true
    }
    "status" {
        $base = "main"
        if ($args.Count -ge 2) {
            $base = $args[1]
        }
        Show-ReleaseStatus -BaseBranch $base
    }
    default {
        throw "Unknown command '$command'. Use: commit, push, pr, merge-pr, merge-release-pr, status."
    }
}
