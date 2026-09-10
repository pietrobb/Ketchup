[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$EventPath,
    [Parameter(Mandatory = $true)]
    [string]$Repository,
    [string]$OutputPath
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$maxEventBytes = 1024 * 1024
$eventPath = [IO.Path]::GetFullPath($EventPath)
if ($Repository -cnotmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$') {
    throw "Repository must be an exact owner/name identity."
}
if (-not (Test-Path $eventPath -PathType Leaf)) {
    throw "Pull-request event file is missing."
}
$eventFile = Get-Item $eventPath
if ($eventFile.Length -le 0 -or $eventFile.Length -gt $maxEventBytes) {
    throw "Pull-request event exceeds its bounded size."
}
try {
    $eventText = [Text.UTF8Encoding]::new($false, $true).GetString([IO.File]::ReadAllBytes($eventPath))
} catch {
    throw "Pull-request event is not valid UTF-8."
}
try {
    $event = $eventText | ConvertFrom-Json
} catch {
    throw "Pull-request event is not valid JSON."
}

$eventRepository = [string]$event.repository.full_name
$headRepository = [string]$event.pull_request.head.repo.full_name
$headIsFork = $event.pull_request.head.repo.fork
$baseSha = [string]$event.pull_request.base.sha
$headSha = [string]$event.pull_request.head.sha
if ($eventRepository -cnotmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$' -or
    $headRepository -cnotmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$' -or
    $headIsFork -isnot [bool] -or
    $baseSha -cnotmatch '^[0-9a-f]{40}$' -or
    $headSha -cnotmatch '^[0-9a-f]{40}$') {
    throw "Pull-request event has an invalid repository or revision identity."
}
if ($eventRepository -cne $Repository) {
    throw "Pull-request event repository does not match GITHUB_REPOSITORY."
}

$allowSelfHosted = -not $headIsFork -and $headRepository -ceq $Repository
$reason = if ($allowSelfHosted) { "trusted-same-repository-head" } else { "disposable-hosted-only" }
$result = [ordered]@{
    allow_self_hosted = $allowSelfHosted
    reason = $reason
    base_sha = $baseSha
    head_sha = $headSha
}

if (-not [string]::IsNullOrWhiteSpace($OutputPath)) {
    $lines = @(
        "allow_self_hosted=$($allowSelfHosted.ToString().ToLowerInvariant())"
        "reason=$reason"
        "base_sha=$baseSha"
        "head_sha=$headSha"
    ) -join [Environment]::NewLine
    [IO.File]::AppendAllText([IO.Path]::GetFullPath($OutputPath), $lines + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
}

$result | ConvertTo-Json -Compress
