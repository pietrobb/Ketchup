# Gate of build-occt.ps1 (as scripts/macos/build-occt.sh): no OCCT library may
# take a transcendental function from the CRT (that is what made results
# platform dependent), and none may export the private crmath copy linked into
# it, which another library could then import instead of its own (PL-65).
param(
    [Parameter(Mandatory = $true)][string]$Bin,
    [Parameter(Mandatory = $true)][string]$Dumpbin
)

$ErrorActionPreference = "Stop"
$math = '((a?(sin|cos|tan)h?|atan2|exp(2|10|m1)?|log(2|10|1p)?|pow|cbrt|_?hypot|erfc?|[lt]gamma)f?|__vdecl_\w+|_?_libm_sse2\w*)'
# /imports: "<hint> <name>"; /exports: "<ordinal> <hint> <RVA> <name>", maybe "= <forward>".
$imported = "^\s+[0-9A-Fa-f]+\s+$math$"
$exported = "^\s+\d+\s+[0-9A-Fa-f]+\s+([0-9A-Fa-f]{8}\s+)?$math(\s|$)"

$libraries = @(Get-ChildItem $Bin -Filter "TK*.dll")
if (-not $libraries) { throw "No OCCT library (TK*.dll) in $Bin." }
$leaks = foreach ($dll in $libraries) {
    & $Dumpbin /nologo /imports $dll.FullName |
        Where-Object { $_ -match $imported } |
        ForEach-Object { "$($dll.Name) imports $($_.Trim())" }
    & $Dumpbin /nologo /exports $dll.FullName |
        Where-Object { $_ -match $exported } |
        ForEach-Object { "$($dll.Name) exports $($_.Trim())" }
}
# Each leak as a plain line of its own: pwsh renders a thrown message wrapped,
# indented and coloured, so a long leak list would not read (or match) as lines.
if ($leaks) {
    $leaks | ForEach-Object { [Console]::Error.WriteLine($_) }
    throw "OCCT math is not its own private crmath: $(@($leaks).Count) leak(s) listed above."
}
