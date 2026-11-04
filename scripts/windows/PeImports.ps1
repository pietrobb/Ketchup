# The DLLs a 64-bit PE image imports, directly and delay-loaded, in lower case.
# build-installer.ps1 refuses a package with an executable or library that needs
# a DLL neither the package nor a clean Windows has (review 2026-11-02 REL-120).
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

# Whether a clean Windows 10 or 11 has the DLL: an API set the loader resolves, or a
# file of Windows itself in the system directory. A redistributable runtime there
# (Visual C++, MFC, OpenMP) came with some other program and is not part of Windows.
function Test-WindowsSystemDll([string]$Name, [string]$SystemDirectory = [Environment]::SystemDirectory) {
    if ($Name -match '^(api|ext)-ms-') { return $true }
    if ([string]::IsNullOrEmpty($SystemDirectory)) { return $false }
    $path = Join-Path $SystemDirectory $Name
    if (-not (Test-Path $path -PathType Leaf)) { return $false }
    return [string](Get-Item $path).VersionInfo.ProductName -match 'Windows'
}

# Every import of every .exe and .dll in $Directory that neither $Directory nor Windows
# has, as "file: dll", sorted.
function Get-UnresolvedPeImports([string]$Directory, [string]$SystemDirectory = [Environment]::SystemDirectory) {
    $images = @(Get-ChildItem $Directory -File | Where-Object { $_.Extension -in '.exe', '.dll' })
    $present = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($image in $images) { [void]$present.Add($image.Name) }
    $system = @{}
    $missing = foreach ($image in $images) {
        foreach ($dll in Get-PeImportedDlls $image.FullName) {
            if ($present.Contains($dll)) { continue }
            if (-not $system.ContainsKey($dll)) { $system[$dll] = Test-WindowsSystemDll $dll $SystemDirectory }
            if (-not $system[$dll]) { "$($image.Name): $dll" }
        }
    }
    return @($missing | Sort-Object -Unique)
}

# Copies from $RuntimeDirectory each DLL the images in $Directory miss, and what those
# DLLs miss in turn; returns the names copied.
function Copy-MissingPeImports([string]$Directory, [string]$RuntimeDirectory, [string]$SystemDirectory = [Environment]::SystemDirectory) {
    $copied = [Collections.Generic.List[string]]::new()
    do {
        $added = 0
        foreach ($dll in @(Get-UnresolvedPeImports $Directory $SystemDirectory | ForEach-Object { ($_ -split ': ', 2)[1] } | Sort-Object -Unique)) {
            $source = Join-Path $RuntimeDirectory $dll
            if (Test-Path $source -PathType Leaf) {
                Copy-Item $source (Join-Path $Directory ([IO.Path]::GetFileName($source)))
                $copied.Add($dll)
                $added++
            }
        }
    } while ($added -ne 0)
    return $copied.ToArray()
}

# The executables and libraries in $Directory that carry no valid signature, by name: what
# a signed package must sign itself, the OCCT TK*.dll as well as the programs (review
# 2026-11-04 REL-142); the Visual C++ runtime keeps Microsoft's own.
function Get-PeFilesToSign([string]$Directory) {
    Get-ChildItem $Directory -File |
        Where-Object { $_.Extension -in ".exe", ".dll" -and (Get-AuthenticodeSignature $_.FullName).Status -ne "Valid" } |
        Sort-Object Name | ForEach-Object { $_.Name }
}

# The Visual C++ runtime DLLs of the newest installed Visual Studio, which Microsoft
# allows to ship beside the program (app-local deployment).
function Get-VcRuntimeDirectory {
    $roots = @()
    if (-not [string]::IsNullOrWhiteSpace($env:VCToolsRedistDir)) { $roots += $env:VCToolsRedistDir }
    $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vswhere -PathType Leaf) {
        foreach ($installation in @(& $vswhere -latest -products * -property installationPath)) {
            $roots += @(Get-ChildItem (Join-Path $installation "VC\Redist\MSVC") -Directory -ErrorAction SilentlyContinue |
                Where-Object { $_.Name -match '^\d+\.\d+\.\d+$' } |
                Sort-Object { [version]$_.Name } -Descending |
                ForEach-Object { $_.FullName })
        }
    }
    foreach ($root in $roots) {
        $crt = @(Get-ChildItem (Join-Path $root "x64") -Directory -Filter "Microsoft.VC*.CRT" -ErrorAction SilentlyContinue | Select-Object -First 1)
        if ($crt.Count -eq 1) { return $crt[0].FullName }
    }
    throw "No Visual C++ x64 runtime to ship found: install the MSVC build tools or set VCToolsRedistDir."
}
