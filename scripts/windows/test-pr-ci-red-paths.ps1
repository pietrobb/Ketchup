[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$runner = Join-Path $PSScriptRoot "invoke-pr-ci-group.ps1"
$policy = Join-Path $PSScriptRoot "get-pr-runner-policy.ps1"
$workflow = Join-Path ([IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))) ".github\workflows\ci.yml"
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("ketchup-pr-ci-red-" + [Guid]::NewGuid().ToString("N"))
$fakeCargo = Join-Path $tempRoot "cargo.cmd"
$failingNativeExecutable = [string](Get-Command where.exe -CommandType Application -ErrorAction Stop).Source

function Write-EventFixture([string]$Path, [string]$Repository, [string]$HeadRepository, [bool]$Fork) {
    $event = [ordered]@{
        repository = [ordered]@{ full_name = $Repository }
        pull_request = [ordered]@{
            base = [ordered]@{ sha = "1" * 40 }
            head = [ordered]@{
                sha = "2" * 40
                repo = [ordered]@{ full_name = $HeadRepository; fork = $Fork }
            }
        }
    }
    [IO.File]::WriteAllText($Path, (($event | ConvertTo-Json -Depth 6) + "`n"), [Text.UTF8Encoding]::new($false))
}

New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null
try {
    $workflowSource = [IO.File]::ReadAllText($workflow)
    $runnerSource = [IO.File]::ReadAllText($runner)
    if ($runnerSource -notmatch [regex]::Escape('Get-Command $CargoCommand -CommandType Application') -or
        $runnerSource -notmatch [regex]::Escape('[IO.FileShare]::Read') -or
        $runnerSource -notmatch [regex]::Escape('$cargoPath.StartsWith($repoPrefix') -or
        $runnerSource -notmatch [regex]::Escape('$cargoGuard.Dispose()') -or
        [regex]::Matches($runnerSource, [regex]::Escape('"--all-features"')).Count -ne 3) {
        throw "PR CI runner does not pin Cargo outside the checkout under a write/delete-blocking handle."
    }
    $trustedCondition = "if: github.event_name == 'pull_request_target' && needs.pull-request-runner-policy.outputs.allow_self_hosted == 'true' && !github.event.pull_request.head.repo.fork && github.event.pull_request.head.repo.full_name == github.repository"
    if ([regex]::Matches($workflowSource, [regex]::Escape("runs-on: [self-hosted, Windows, X64, ketchup-occt-r0-v1]")).Count -ne 5 -or
        [regex]::Matches($workflowSource, [regex]::Escape("needs: pull-request-runner-policy")).Count -ne 4 -or
        [regex]::Matches($workflowSource, [regex]::Escape($trustedCondition)).Count -ne 4 -or
        [regex]::Matches($workflowSource, [regex]::Escape('ref: ${{ github.event.pull_request.head.sha }}')).Count -ne 4 -or
        $workflowSource -notmatch [regex]::Escape('group: ci-governance-${{ github.event_name }}-${{ github.event.pull_request.number || github.ref }}') -or
        $workflowSource -notmatch [regex]::Escape("pull_request_target:") -or
        $workflowSource -notmatch [regex]::Escape("ref: `${{ github.event.pull_request.base.sha }}")) {
        throw "PR workflow does not keep every self-hosted job behind the trusted-base same-repository policy."
    }

    $trustedEvent = Join-Path $tempRoot "trusted.json"
    $trustedOutput = Join-Path $tempRoot "trusted-output.txt"
    Write-EventFixture $trustedEvent "owner/ketchup" "owner/ketchup" $false
    $trusted = (& $policy -EventPath $trustedEvent -Repository "owner/ketchup" -OutputPath $trustedOutput) | ConvertFrom-Json
    if ($trusted.allow_self_hosted -ne $true -or
        (Get-Content $trustedOutput -Raw) -notmatch '(?m)^allow_self_hosted=true\r?$') {
        throw "Same-repository PR did not receive the trusted self-hosted classification."
    }

    foreach ($case in @(
        [ordered]@{ name = "fork"; repository = "contributor/ketchup"; fork = $true },
        [ordered]@{ name = "spoofed-non-fork"; repository = "contributor/ketchup"; fork = $false },
        [ordered]@{ name = "spoofed-fork-flag"; repository = "owner/ketchup"; fork = $true }
    )) {
        $eventPath = Join-Path $tempRoot ($case.name + ".json")
        Write-EventFixture $eventPath "owner/ketchup" $case.repository $case.fork
        $classification = (& $policy -EventPath $eventPath -Repository "owner/ketchup") | ConvertFrom-Json
        if ($classification.allow_self_hosted -ne $false -or $classification.reason -cne "disposable-hosted-only") {
            throw "Untrusted PR case '$($case.name)' reached the self-hosted runner."
        }
    }
    Write-Output "PR runner policy denied all external/spoofed heads and allowed only an exact same-repository non-fork head."

    [IO.File]::WriteAllText(
        $fakeCargo,
        "@echo off`r`nexit /b 17`r`n",
        [Text.ASCIIEncoding]::new()
    )
    $shimOutput = @()
    $shimExitCode = 0
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $shimOutput = @(& powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $runner -Group quality-build -CargoCommand $fakeCargo 2>&1)
        $shimExitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousPreference
    }
    if ($shimExitCode -eq 0 -or ($shimOutput -join "`n") -notmatch "native executable outside the untrusted checkout") {
        throw "PR CI runner accepted a command shim from the untrusted checkout."
    }

    foreach ($group in @("quality-build", "foundations", "scheduler", "app")) {
        $output = @()
        $exitCode = 0
        $previousPreference = $ErrorActionPreference
        try {
            $ErrorActionPreference = "Continue"
            $output = @(& powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $runner -Group $group -CargoCommand $failingNativeExecutable 2>&1)
            $exitCode = $LASTEXITCODE
        } finally {
            $ErrorActionPreference = $previousPreference
        }
        if ($exitCode -eq 0) {
            throw "Expected PR CI group '$group' to propagate a failing Cargo step."
        }
        if (($output -join "`n") -notmatch "failed with exit code [1-9][0-9]*") {
            throw "PR CI group '$group' failed for the wrong reason: $($output -join ' ')"
        }
        Write-Output "RED confirmed: $group propagates Cargo failure"
    }

    Write-Output "All PR CI groups propagated their deliberate-red Cargo failure."
} finally {
    if (Test-Path $tempRoot) { Remove-Item $tempRoot -Recurse -Force }
}
