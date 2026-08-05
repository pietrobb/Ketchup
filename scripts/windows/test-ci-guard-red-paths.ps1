[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$guardScript = Join-Path $PSScriptRoot "test-architecture-guards.ps1"
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("ketchup-ci-guard-selftest-" + [Guid]::NewGuid().ToString("N"))

function Write-Utf8NoBom([string]$Path, [string]$Content) {
    $parent = Split-Path $Path -Parent
    if (-not (Test-Path $parent)) { New-Item -ItemType Directory -Path $parent -Force | Out-Null }
    [IO.File]::WriteAllText($Path, $Content, [Text.UTF8Encoding]::new($false))
}

function Invoke-ExpectedRed([string]$Name, [string]$ExpectedGuard, [scriptblock]$Arrange, [string[]]$ChangedPaths = @()) {
    & $Arrange
    $output = @()
    try {
        $arguments = @(
            "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", $guardScript,
            "-RepoRoot", $tempRoot,
            "-ChangeManifestPath", (Join-Path $tempRoot "governance\contract-changes.json"),
            "-FrozenLockPath", (Join-Path $tempRoot "governance\frozen-inputs.json")
        )
        if ($ChangedPaths.Count -gt 0) { $arguments += @("-ChangedPaths") + $ChangedPaths }
        $output = @(& powershell.exe @arguments 2>&1)
        if ($LASTEXITCODE -eq 0) { throw "Expected guard $ExpectedGuard to reject $Name, but it passed." }
    } catch {
        $output += $_.Exception.Message
    }
    if (($output -join "`n") -notmatch [regex]::Escape("[guard:$ExpectedGuard]")) {
        throw "Red-path $Name failed for the wrong reason: $($output -join ' ')"
    }
    Write-Output "RED confirmed: $Name -> $ExpectedGuard"
}

function Invoke-ExpectedTransitionRed([string]$Name, [scriptblock]$Arrange, [string]$ExpectedPattern) {
    & $Arrange
    $validator = Join-Path $repoRoot "scripts\windows\validate-r0-transition-classifications.ps1"
    $ledger = Join-Path $tempRoot "governance\r0-transitions-v1-v13.json"
    $output = @()
    $exitCode = 0
    try {
        $output = @(& powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $validator `
            -RepoRoot $repoRoot -LedgerPath $ledger 2>&1)
        $exitCode = $LASTEXITCODE
    } catch {
        $output += $_.Exception.Message
        $exitCode = 1
    }
    if ($exitCode -eq 0) { throw "Expected R0 transition validator to reject $Name, but it passed." }
    if (($output -join "`n") -notmatch $ExpectedPattern) {
        throw "R0 transition red-path $Name failed for the wrong reason: $($output -join ' ')"
    }
    Write-Output "RED confirmed: $Name -> r0-transition-governance"
}

New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null
try {
    $documentSource = Get-Content (Join-Path $repoRoot "crates\ketchup-core\src\document.rs") -Raw
    $d08Source = Get-Content (Join-Path $repoRoot "governance\d08-lifecycle-exceptions.json") -Raw
    $transitionSource = Get-Content (Join-Path $repoRoot "governance\r0-transitions-v1-v13.json") -Raw
    $appSource = Get-Content (Join-Path $repoRoot "crates\ketchup-app\src\lib.rs") -Raw
    $exactManifestSource = Get-Content (Join-Path $repoRoot "crates\ketchup-exact\Cargo.toml") -Raw
    $ciGovernanceSource = Get-Content (Join-Path $repoRoot "scripts\windows\invoke-ci-governance.ps1") -Raw
    $a0V1RunnerSource = Get-Content (Join-Path $repoRoot "scripts\windows\run-strengthened-a0-v1.ps1") -Raw
    $a0V2RunnerSource = Get-Content (Join-Path $repoRoot "scripts\windows\run-strengthened-a0-v2.ps1") -Raw
    $stateViewSource = "pub const COMPLETE_STATE_VIEW_V1: &str = `"complete`";`npub const AGENT_STATE_VIEW_V1: &str = `"agent`";`nfn encode_semantic_state() {}`n"
    Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-core\src\document.rs") $documentSource
    Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-core\src\state_view.rs") $stateViewSource
    Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-core\tests\product_document.rs") "// D-08 lifecycle evidence fixture`n"
    Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-app\tests\file_workflow.rs") "// D-08 Open/New evidence fixture`n"
    Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-core\tests\fixtures\state-view\complete-v1.txt") "complete`n"
    Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-core\tests\fixtures\state-view\agent-v1.txt") "agent`n"
    Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-app\src\lib.rs") $appSource
    Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-exact\Cargo.toml") $exactManifestSource
    Write-Utf8NoBom (Join-Path $tempRoot "scripts\windows\invoke-ci-governance.ps1") $ciGovernanceSource
    Write-Utf8NoBom (Join-Path $tempRoot "scripts\windows\run-strengthened-a0-v1.ps1") $a0V1RunnerSource
    Write-Utf8NoBom (Join-Path $tempRoot "scripts\windows\run-strengthened-a0-v2.ps1") $a0V2RunnerSource
    Write-Utf8NoBom (Join-Path $tempRoot "governance\d08-lifecycle-exceptions.json") $d08Source
    Write-Utf8NoBom (Join-Path $tempRoot "governance\contract-changes.json") "{`n  `"schema_version`": 1,`n  `"changes`": []`n}`n"
    Write-Utf8NoBom (Join-Path $tempRoot "governance\frozen.txt") "frozen`n"
    $frozenHash = (Get-FileHash (Join-Path $tempRoot "governance\frozen.txt") -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-Utf8NoBom (Join-Path $tempRoot "governance\frozen-inputs.json") ("{`n  `"files`": [{`"path`": `"governance/frozen.txt`", `"sha256`": `"$frozenHash`"}]`n}`n")

    $baselineOutput = @(& powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $guardScript `
        -RepoRoot $tempRoot `
        -ChangeManifestPath (Join-Path $tempRoot "governance\contract-changes.json") `
        -FrozenLockPath (Join-Path $tempRoot "governance\frozen-inputs.json") 2>&1)
    if ($LASTEXITCODE -ne 0) { throw "Self-test fixture baseline is invalid: $($baselineOutput -join ' ')" }

    Invoke-ExpectedRed "sealed A0 target returned to daily test discovery" "a0-separation" {
        $mutated = $exactManifestSource.Replace('required-features = ["a0-certification"]', 'required-features = []')
        Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-exact\Cargo.toml") $mutated
    }
    Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-exact\Cargo.toml") $exactManifestSource

    Invoke-ExpectedRed "public associated mutator" "sole-mutation" {
        $mutated = $documentSource.Replace(
            "impl DocumentStore {",
            "impl DocumentStore {`n    pub const fn bypass_gateway(store: &mut Store) { store.cursor = 0; }"
        )
        Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-core\src\document.rs") $mutated
    }
    Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-core\src\document.rs") $documentSource

    Invoke-ExpectedRed "private in-core revision write bypass" "sole-mutation" {
        $mutated = $documentSource.Replace(
            "impl DocumentStore {",
            "impl DocumentStore {`n    fn bypass_inside_core(&mut self) { self.revisions[self.cursor] = self.revisions[self.cursor].clone(); }"
        )
        Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-core\src\document.rs") $mutated
    }
    Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-core\src\document.rs") $documentSource

    Invoke-ExpectedRed "candidate mutation after validation" "sole-mutation" {
        $mutated = $documentSource.Replace(
            "let recomputed_nodes =",
            "product = product.clone();`n        let recomputed_nodes ="
        )
        Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-core\src\document.rs") $mutated
    }
    Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-core\src\document.rs") $documentSource

    Invoke-ExpectedRed "canonical mutation hidden inside Undo" "sole-mutation" {
        $mutated = $documentSource.Replace("self.cursor -= 1;", "self.cursor -= 1;`n        self.next_revision_id += 1;")
        Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-core\src\document.rs") $mutated
    }
    Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-core\src\document.rs") $documentSource

    Invoke-ExpectedRed "public persistence constructor" "sole-mutation" {
        $mutated = $documentSource.Replace("pub(crate) fn from_product", "pub fn from_product")
        Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-core\src\document.rs") $mutated
    }
    Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-core\src\document.rs") $documentSource

    Invoke-ExpectedRed "unreviewed lifecycle exception declaration" "sole-mutation" {
        $mutated = $d08Source.Replace('"kind": "cursor_only"', '"kind": "entity_mutation"')
        Write-Utf8NoBom (Join-Path $tempRoot "governance\d08-lifecycle-exceptions.json") $mutated
    }
    Write-Utf8NoBom (Join-Path $tempRoot "governance\d08-lifecycle-exceptions.json") $d08Source

    Invoke-ExpectedRed "legacy scene authority" "legacy-absence" {
        Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-app\src\legacy.rs") "pub struct SceneBox;`n"
    }
    Remove-Item (Join-Path $tempRoot "crates\ketchup-app\src\legacy.rs") -Force

    Invoke-ExpectedRed "direct production interaction scene construction" "projection-authority" {
        Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-app\src\lib.rs") ($appSource + "`nfn bypass() { InteractionScene::new(); }`n")
    }
    Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-app\src\lib.rs") $appSource

    Invoke-ExpectedRed "missing StateView golden evidence" "state-view" {
        Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-core\tests\fixtures\state-view\agent-v1.txt") ""
    }
    Write-Utf8NoBom (Join-Path $tempRoot "crates\ketchup-core\tests\fixtures\state-view\agent-v1.txt") "agent`n"

    Invoke-ExpectedRed "stale frozen input" "frozen-input" {
        Write-Utf8NoBom (Join-Path $tempRoot "governance\frozen.txt") "tampered`n"
    }
    Write-Utf8NoBom (Join-Path $tempRoot "governance\frozen.txt") "frozen`n"

    Invoke-ExpectedRed "dot-prefixed protected path with stale reviewed hash" "anti-loosening" {
        Write-Utf8NoBom (Join-Path $tempRoot ".github\CODEOWNERS") "/governance/ @owner`n"
        Write-Utf8NoBom (Join-Path $tempRoot "governance\contract-changes.json") @'
{
  "schema_version": 1,
  "changes": [
    {
      "path": ".github/CODEOWNERS",
      "direction": "tighten",
      "old_freeze_id": "old",
      "new_freeze_id": "new",
      "new_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
      "evidence": "governance/frozen.txt"
    }
  ]
}
'@
    } @(".github/CODEOWNERS")

    Invoke-ExpectedRed "unclassified threshold change" "anti-loosening" {} @("thresholds/r0.yaml")

    Invoke-ExpectedRed "unapproved threshold loosening" "anti-loosening" {
        Write-Utf8NoBom (Join-Path $tempRoot "governance\contract-changes.json") @'
{
  "schema_version": 1,
  "changes": [
    {
      "path": "thresholds/r0.yaml",
      "direction": "loosen",
      "old_freeze_id": "r0-v1",
      "new_freeze_id": "r0-v2",
      "evidence": "governance/frozen.txt",
      "approval": "",
      "upper_envelope_evidence": ""
    }
  ]
}
'@
    } @("thresholds/r0.yaml")

    Invoke-ExpectedTransitionRed "unknown historical transition direction" {
        $ledger = $transitionSource | ConvertFrom-Json
        $ledger.transitions[0].direction = "unknown"
        Write-Utf8NoBom (Join-Path $tempRoot "governance\r0-transitions-v1-v13.json") ($ledger | ConvertTo-Json -Depth 20)
    } "unknown or invalid direction"

    Invoke-ExpectedTransitionRed "deleted upper-envelope case" {
        $ledger = $transitionSource | ConvertFrom-Json
        $ledger.upper_envelope_coverage.cases = @($ledger.upper_envelope_coverage.cases | Select-Object -First 4)
        Write-Utf8NoBom (Join-Path $tempRoot "governance\r0-transitions-v1-v13.json") ($ledger | ConvertTo-Json -Depth 20)
    } "retain all five classified cases"

    Write-Output "All deliberate-red architecture and R0 transition guard paths were observed exactly once."
} finally {
    if (Test-Path $tempRoot) { Remove-Item $tempRoot -Recurse -Force }
}
