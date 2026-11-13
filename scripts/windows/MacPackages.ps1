# A release has both macOS packages, the app and the Viewer, or neither: one alone
# gave sums with the Viewer's line but not the app's, and a package given twice gave
# its line twice (review 2026-11-13 P3).

function Assert-MacPackages([string]$Version, [string[]]$Packages) {
    if (-not $Packages) { return }
    $names = @("Ketchup-$Version-macOS.dmg", "Ketchup-Viewer-$Version-macOS.dmg")
    foreach ($package in $Packages) {
        if (-not (Test-Path -LiteralPath $package -PathType Leaf)) {
            throw "The macOS package $package does not exist."
        }
        if ($names -cnotcontains [IO.Path]::GetFileName($package)) {
            throw "$package is not a macOS package of $Version; the release names them $($names -join ' and ')."
        }
    }
    $given = @($Packages | ForEach-Object { [IO.Path]::GetFileName($_) } | Sort-Object)
    if ($given.Count -ne 2 -or $given[0] -ceq $given[1]) {
        throw "-MacPackages gave $($given -join ', '); a release has both $($names -join ' and '), each once, or neither."
    }
}
