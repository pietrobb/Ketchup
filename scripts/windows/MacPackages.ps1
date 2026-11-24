# A release has both macOS packages, the app and the Viewer, or neither: one alone
# gave sums with the Viewer's line but not the app's, and a package given twice gave
# its line twice (review 2026-11-13 P3).
#
# Each is signed with the Developer ID of CENTAURY Plus s.r.o. and carries its
# notarization ticket stapled (review 2026-11-13 r95 P3: notarization was only written
# down). The last 512 bytes of a disk image, its UDIF trailer `koly`, give the offset and
# size of its code signature at 0x128 and 0x130; that is an embedded-signature blob whose
# slot 0x10000 holds the CMS signature and slot 0x10002 the ticket `xcrun stapler staple`
# adds, which starts with "s8ch" (so are both 0.3.3 packages). Gatekeeper checks the
# ticket itself; this refuses a package signed by another or never stapled.

$MacSigner = "Developer ID Application: CENTAURY Plus s.r.o. (HGHK355YL8)"

function Read-BigEndian([byte[]]$Bytes, [long]$At, [int]$Size) {
    [long]$value = 0
    for ($i = 0; $i -lt $Size; $i++) { $value = ($value -shl 8) -bor $Bytes[$At + $i] }
    $value
}

# The blobs of a disk image's code signature by slot, or $null if it has none.
function Get-DmgSignatureSlots([string]$Path) {
    $reader = New-Object IO.BinaryReader ([IO.File]::OpenRead((Resolve-Path -LiteralPath $Path).ProviderPath))
    try {
        $length = $reader.BaseStream.Length
        if ($length -lt 512) { return $null }
        $reader.BaseStream.Seek(-512, [IO.SeekOrigin]::End) | Out-Null
        $trailer = $reader.ReadBytes(512)
        if ([Text.Encoding]::ASCII.GetString($trailer, 0, 4) -cne "koly") { return $null }
        $offset = Read-BigEndian $trailer 0x128 8
        $size = Read-BigEndian $trailer 0x130 8
        if ($size -lt 12 -or $size -gt 1MB -or $offset -lt 0 -or $offset + $size -gt $length - 512) {
            return $null
        }
        $reader.BaseStream.Seek($offset, [IO.SeekOrigin]::Begin) | Out-Null
        $signature = $reader.ReadBytes([int]$size)
    } finally {
        $reader.Dispose()
    }
    if ((Read-BigEndian $signature 0 4) -ne 0xfade0cc0L) { return $null }
    $count = Read-BigEndian $signature 8 4
    if (12 + 8 * $count -gt $size) { return $null }
    $slots = @{}
    for ($i = 0; $i -lt $count; $i++) {
        $type = Read-BigEndian $signature (12 + 8 * $i) 4
        $at = Read-BigEndian $signature (16 + 8 * $i) 4
        if ($at + 8 -gt $size) { return $null }
        $blob = Read-BigEndian $signature ($at + 4) 4
        if ($blob -lt 8 -or $at + $blob -gt $size) { return $null }
        $slots[$type] = [byte[]]$signature[($at + 8)..($at + $blob - 1)]
    }
    $slots
}

function Assert-NotarizedPackage([string]$Path) {
    $slots = Get-DmgSignatureSlots $Path
    $signed = if ($slots) { $slots[[long]0x10000] }
    if (-not $signed -or [Text.Encoding]::ASCII.GetString($signed).IndexOf($MacSigner, [StringComparison]::Ordinal) -lt 0) {
        throw "$Path is not signed with $MacSigner; sign it on the Mac (README, macOS)."
    }
    $ticket = $slots[[long]0x10002]
    if (-not $ticket -or $ticket.Length -lt 4 -or [Text.Encoding]::ASCII.GetString($ticket, 0, 4) -cne "s8ch") {
        throw "$Path carries no notarization ticket; notarize it and staple the ticket on the Mac (xcrun notarytool submit --wait, xcrun stapler staple; README, macOS)."
    }
}

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
    foreach ($package in $Packages) {
        Assert-NotarizedPackage $package
    }
}
