# The DLLs a 64-bit PE image imports, directly and delay-loaded, in lower case.
# build-installer.ps1 refuses a Viewer that needs a DLL a clean Windows lacks.
function Get-PeImportedDlls([string]$Path) {
    $image = [IO.File]::ReadAllBytes($Path)
    $pe = [BitConverter]::ToInt32($image, 0x3c)
    if ([BitConverter]::ToUInt32($image, $pe) -ne 0x00004550) { throw "Not a PE image: $Path" }
    $sections = [BitConverter]::ToUInt16($image, $pe + 6)
    $optionalSize = [BitConverter]::ToUInt16($image, $pe + 20)
    $optional = $pe + 24
    if ([BitConverter]::ToUInt16($image, $optional) -ne 0x20b) { throw "Not a PE32+ image: $Path" }
    $sectionTable = $optional + $optionalSize
    $offsetOf = {
        param([uint32]$Rva)
        for ($index = 0; $index -lt $sections; $index++) {
            $header = $sectionTable + 40 * $index
            $start = [BitConverter]::ToUInt32($image, $header + 12)
            $size = [Math]::Max([BitConverter]::ToUInt32($image, $header + 8), [BitConverter]::ToUInt32($image, $header + 16))
            if ($Rva -ge $start -and $Rva -lt $start + $size) {
                return [int]($Rva - $start + [BitConverter]::ToUInt32($image, $header + 20))
            }
        }
        throw "RVA $Rva lies in no section of $Path"
    }
    $dlls = [Collections.Generic.List[string]]::new()
    # Data directory 1 holds the imports (20-byte descriptors, name at +12),
    # 13 the delay-loaded ones (32-byte descriptors, name at +4).
    foreach ($directory in @(@{ index = 1; entry = 20; name = 12 }, @{ index = 13; entry = 32; name = 4 })) {
        $rva = [BitConverter]::ToUInt32($image, $optional + 112 + 8 * $directory.index)
        if ($rva -eq 0) { continue }
        $at = & $offsetOf $rva
        while (@($image[$at..($at + $directory.entry - 1)] | Where-Object { $_ -ne 0 }).Count -ne 0) {
            $nameAt = & $offsetOf ([BitConverter]::ToUInt32($image, $at + $directory.name))
            $end = [Array]::IndexOf($image, [byte]0, $nameAt)
            $dlls.Add([Text.Encoding]::ASCII.GetString($image, $nameAt, $end - $nameAt).ToLowerInvariant())
            $at += $directory.entry
        }
    }
    return $dlls.ToArray()
}
