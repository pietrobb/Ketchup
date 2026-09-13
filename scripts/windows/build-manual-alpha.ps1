[CmdletBinding()]
param(
    [string]$Version = "0.1.0-alpha.1",
    [string]$OutputDir,
    [string]$BinaryDir,
    [string]$OcctRoot,
    [string]$OcctManifestPath,
    [string]$InnoCompilerPath,
    [string]$SigningMetadataPath,
    [string]$SignToolPath,
    [string]$AzureSigningDlibPath,
    [switch]$Sign,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path $repoRoot "artifacts\manual-alpha\$Version"
}
if ([string]::IsNullOrWhiteSpace($BinaryDir)) {
    $BinaryDir = Join-Path $repoRoot "target\release"
}
if ([string]::IsNullOrWhiteSpace($OcctRoot)) {
    $OcctRoot = Join-Path $repoRoot "third_party\occt-install-r0-v1"
}
if ([string]::IsNullOrWhiteSpace($OcctManifestPath)) {
    $OcctManifestPath = Join-Path $repoRoot "artifacts\r0\occt-build-manifest.json"
}
if ([string]::IsNullOrWhiteSpace($InnoCompilerPath)) {
    $candidates = @(
        (Join-Path $env:LOCALAPPDATA "Programs\Inno Setup 6\ISCC.exe"),
        "C:\Program Files (x86)\Inno Setup 6\ISCC.exe",
        "C:\Program Files\Inno Setup 6\ISCC.exe"
    )
    $InnoCompilerPath = @($candidates | Where-Object { Test-Path $_ -PathType Leaf } | Select-Object -First 1)
}
if ([string]::IsNullOrWhiteSpace($SigningMetadataPath)) {
    $SigningMetadataPath = Join-Path $PSScriptRoot "signing\metadata.json"
}

$OutputDir = [IO.Path]::GetFullPath($OutputDir)
$BinaryDir = [IO.Path]::GetFullPath($BinaryDir)
$OcctRoot = [IO.Path]::GetFullPath($OcctRoot)
$OcctManifestPath = [IO.Path]::GetFullPath($OcctManifestPath)
$SigningMetadataPath = [IO.Path]::GetFullPath($SigningMetadataPath)
$payloadDir = Join-Path $OutputDir "payload"
$setupBaseName = "Ketchup-Manual-Alpha-$Version-Setup"
$setupPath = Join-Path $OutputDir "$setupBaseName.exe"
$zipPath = Join-Path $OutputDir "Ketchup-Manual-Alpha-$Version-Portable.zip"
$packageManifestPath = Join-Path $payloadDir "package-manifest.json"

function Assert-Leaf([string]$Path, [string]$Label) {
    if (-not (Test-Path $Path -PathType Leaf)) {
        throw "Missing $Label`: $Path"
    }
}

function Get-Sha256([string]$Path) {
    return (Get-FileHash $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Write-Utf8([string]$Path, [string]$Text) {
    [IO.File]::WriteAllText($Path, $Text, [Text.UTF8Encoding]::new($false))
}

function Get-RelativePath([string]$Root, [string]$Path) {
    return [IO.Path]::GetRelativePath($Root, $Path).Replace("\", "/")
}

function Resolve-SigningTool([string]$ConfiguredPath, [string]$PackageName, [string]$FileName) {
    if (-not [string]::IsNullOrWhiteSpace($ConfiguredPath)) {
        $resolved = [IO.Path]::GetFullPath($ConfiguredPath)
        Assert-Leaf $resolved $FileName
        return $resolved
    }

    $packageRoot = Join-Path $env:USERPROFILE ".nuget\packages\$PackageName"
    $resolved = @(Get-ChildItem $packageRoot -Filter $FileName -File -Recurse -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match '[\\/]x64[\\/]' } |
        Sort-Object FullName -Descending |
        Select-Object -First 1)
    if ($resolved.Count -ne 1) {
        throw "Unable to locate x64 $FileName in $packageRoot."
    }
    return $resolved[0].FullName
}

function Invoke-AzureSigning([string]$Path) {
    & $SignToolPath sign /v /fd SHA256 /tr "http://timestamp.acs.microsoft.com" /td SHA256 /dlib $AzureSigningDlibPath /dmdf $SigningMetadataPath $Path
    if ($LASTEXITCODE -ne 0) { throw "Azure Artifact Signing failed: $Path" }

    & $SignToolPath verify /pa /v $Path
    if ($LASTEXITCODE -ne 0) { throw "Authenticode verification failed: $Path" }

    $signature = Get-AuthenticodeSignature $Path
    if ($signature.Status -ne [Management.Automation.SignatureStatus]::Valid -or
        $null -eq $signature.SignerCertificate -or
        $signature.SignerCertificate.Subject -notlike 'CN=AEON GROUP spol. s r.o.*' -or
        $null -eq $signature.TimeStamperCertificate) {
        throw "Unexpected signer or missing timestamp: $Path"
    }
}

if ($env:OS -ne "Windows_NT") {
    throw "Manual Alpha packaging is Windows x64 only."
}
if ($Version -notmatch '^[0-9A-Za-z][0-9A-Za-z.-]{0,63}$') {
    throw "Version contains unsupported characters: $Version"
}
Assert-Leaf $OcctManifestPath "pinned OCCT manifest"
Assert-Leaf $InnoCompilerPath "Inno Setup compiler"
if ($Sign) {
    Assert-Leaf $SigningMetadataPath "Azure signing metadata"
    $SignToolPath = Resolve-SigningTool $SignToolPath "microsoft.windows.sdk.buildtools" "signtool.exe"
    $AzureSigningDlibPath = Resolve-SigningTool $AzureSigningDlibPath "microsoft.trusted.signing.client" "Azure.CodeSigning.Dlib.dll"
    & az account show *> $null
    if ($LASTEXITCODE -ne 0) {
        throw "Azure CLI is not logged in. Run az login before requesting a signed package."
    }
}

if (Test-Path $OutputDir) {
    if (@(Get-ChildItem $OutputDir -Force).Count -ne 0) {
        throw "OutputDir must be absent or empty; refusing to overwrite Manual Alpha artifacts: $OutputDir"
    }
} else {
    [void](New-Item $OutputDir -ItemType Directory)
}
[void](New-Item $payloadDir -ItemType Directory)

if (-not $SkipBuild) {
    Push-Location $repoRoot
    try {
        $previousBuildVersion = [Environment]::GetEnvironmentVariable("KETCHUP_BUILD_VERSION", "Process")
        try {
            [Environment]::SetEnvironmentVariable("KETCHUP_BUILD_VERSION", $Version, "Process")
            & cargo build --locked --release -p ketchup-app --bin ketchup-app --no-default-features --features manual-alpha
            if ($LASTEXITCODE -ne 0) { throw "Manual Alpha application build failed." }
        } finally {
            [Environment]::SetEnvironmentVariable("KETCHUP_BUILD_VERSION", $previousBuildVersion, "Process")
        }
        & cargo build --locked --release -p ketchup-scheduler --bin ketchup-exact-worker
        if ($LASTEXITCODE -ne 0) { throw "Exact worker release build failed." }
    } finally {
        Pop-Location
    }
}

$appSource = Join-Path $BinaryDir "ketchup-app.exe"
$workerSource = Join-Path $BinaryDir "ketchup-exact-worker.exe"
Assert-Leaf $appSource "Manual Alpha application"
Assert-Leaf $workerSource "exact worker"
Copy-Item $appSource (Join-Path $payloadDir "Ketchup.exe")
Copy-Item $workerSource (Join-Path $payloadDir "ketchup-exact-worker.exe")

$occtManifest = Get-Content $OcctManifestPath -Raw | ConvertFrom-Json
if ($occtManifest.schema_version -ne 1 -or
    $occtManifest.status -ne "built-and-fingerprinted" -or
    $occtManifest.build.platform -ne "windows-x86_64" -or
    $occtManifest.build.configuration -ne "Release") {
    throw "OCCT manifest does not describe the pinned Windows x64 Release runtime."
}
foreach ($record in @($occtManifest.shared_libraries)) {
    $relative = [string]$record.path
    if ($relative -notmatch '^win64/vc14/bin/(TK[A-Za-z0-9]+\.dll)$') {
        throw "Unsafe OCCT runtime path in manifest: $relative"
    }
    $name = $Matches[1]
    $source = [IO.Path]::GetFullPath((Join-Path $OcctRoot $relative))
    Assert-Leaf $source "pinned OCCT runtime"
    if ((Get-Item $source).Length -ne [int64]$record.size_bytes -or
        (Get-Sha256 $source) -cne [string]$record.sha256) {
        throw "Pinned OCCT runtime fingerprint mismatch: $name"
    }
    Copy-Item $source (Join-Path $payloadDir $name)
}
if ($Sign) {
    Invoke-AzureSigning (Join-Path $payloadDir "Ketchup.exe")
    Invoke-AzureSigning (Join-Path $payloadDir "ketchup-exact-worker.exe")
}
$publisher = if ($Sign) { "AEON GROUP spol. s r.o." } else { "Ketchup" }
$distributionNotice = if ($Sign) {
    "Ide o digitalne podpisany vyvojovy alpha build od AEON GROUP spol. s r.o., nie verejne vydanie."
} else {
    "Ide o nepodpisany vyvojovy alpha build, nie verejne vydanie. Windows SmartScreen moze pri prvom spusteni zobrazit upozornenie."
}

$startHere = @"
KECUP MANUAL ALPHA $Version
===========================

Tento build je urceny iba na testovanie rucneho 2D/3D modelovania.
AI Assistant je v tomto builde skryty a nie je potrebny ucet, API kluc ani Python.

ODPORUCANY TEST (priblizne 30-60 minut)
1. Spustite Ketchup Manual Alpha a vytvorte prazdny dokument.
2. Vyskusajte pohyb pohladu, zoom, vyber a zrusenie vyberu objektu.
3. Nakreslite obdlznik, kruh a oblukovy profil a zadajte presne rozmery. Kruh sa musi
   zobrazit ako kruh. Oblukovy profil je plocha uzavreta tetivou a musi sa tak zobrazit uz v 2D.
4. Vytvorte z kazdeho profilu 3D teleso pomocou Push/Pull. Vysledok musi zodpovedat
   viditelnemu 2D profilu: valec z kruhu a teleso s oblukom a tetivou z oblukoveho profilu.
5. Vytvorte dve ciastocne sa prekryvajuce 3D telesa. V menu Model vyskusajte Solid Tools:
   Odcitat, Orezat (ponechat rezny objekt), Zjednotit, Prienik a Rozdelit. Pri kazdom nastroji
   kliknite najprv na cielove teleso, potom na druhe teleso a potvrdte Enterom. Medzi pokusmi
   pouzite Undo. Ctrl pri druhom kliknuti zachova nastroj aj pri Odcitani, Zjednoteni a Prieniku;
   Orezanie a Rozdelenie ho zachovavaju automaticky.
6. Zmente rozmer, polohu alebo jednu dostupnu vlastnost vytvoreneho telesa.
7. Vyskusajte Move, Copy, Group/Component alebo pracu s viacerymi objektmi.
8. Ulozte dokument, zavrite ho, znovu ho otvorte a skontrolujte vysledok.
9. Ak je to pre vas relevantne, vyskusajte export do STEP alebo STL.

Nesnazte sa prisposobit aplikacii. Postupujte tak, ako by ste ocakavali od bezneho CAD
programu. Najcennejsie su miesta, kde ste nevedeli, co urobit dalej, alebo kde funkcia,
ktoru ste prirodzene hladali, chybala.

AKO POSLAT SPATNU VAZBU
Vyplnte subor FEEDBACK.txt. Ku kazdemu problemu idealne prilozte screenshot a dokument
.ketchup, na ktorom sa problem prejavil. Neuvadza sa ziadne heslo ani API kluc.

UPOZORNENIE
$distributionNotice
Na dolezitu pracu pouzivajte kopie suborov.
"@
Write-Utf8 (Join-Path $payloadDir "START_HERE.txt") ($startHere.TrimStart() + "`r`n")

$feedback = @"
KECUP MANUAL ALPHA - SPATNA VAZBA
=================================

Tester:
Datum:
Windows verzia:
Rozlisenie / mierka obrazovky:
Predchadzajuce skusenosti s CAD:

A. CELKOVY DOJEM
Co sa vam podarilo vytvorit?

Co bolo zrozumitelne bez vysvetlenia?

Kde ste sa prvykrat zasekli?

B. KONKRETNE PROBLEMY ALEBO CHYBAJUCE FUNKCIE
Pre kazdy bod skopirujte tento blok:

[Problem cislo]
Co som chcel urobit:
Co som presne skusil:
Co sa skutocne stalo:
Co som ocakaval:
Ako dolezite je to pre moju pracu (kriticke / dolezite / prijemne mat):
Da sa problem zopakovat? (vzdy / niekedy / iba raz):
Prilozeny screenshot alebo .ketchup subor:

C. NAJDOLEZITEJSIE DOPLNENIA
1.
2.
3.

D. ZAVERECNE HODNOTENIE
Vedeli by ste dnes v Kecupe dokoncit jednoduchu realnu ulohu? Preco ano alebo nie?

Co by sa muselo zmenit, aby ste ho chceli pouzit znova?
"@
Write-Utf8 (Join-Path $payloadDir "FEEDBACK.txt") ($feedback.TrimStart() + "`r`n")
Write-Utf8 (Join-Path $payloadDir "VERSION.txt") ("Ketchup Manual Alpha $Version`r`nWindows x64`r`nManual-only test build`r`n")

$head = (& git -C $repoRoot rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $head -notmatch '^[0-9a-f]{40}$') {
    throw "Unable to identify the source revision."
}
$dirtyEntries = @(& git -C $repoRoot status --porcelain=v1 --untracked-files=all)
if ($LASTEXITCODE -ne 0) { throw "Unable to inspect the source worktree." }

$files = @(
    Get-ChildItem $payloadDir -File | Sort-Object Name | ForEach-Object {
        [ordered]@{
            path = $_.Name
            size_bytes = $_.Length
            sha256 = Get-Sha256 $_.FullName
        }
    }
)
$packageManifest = [ordered]@{
    schema_version = 1
    kind = "manual-alpha"
    version = $Version
    platform = "windows-x86_64"
    build_features = @("manual-alpha")
    assistant_visible = $false
    code_signing = [ordered]@{
        signed = [bool]$Sign
        publisher = if ($Sign) { $publisher } else { $null }
        timestamped = [bool]$Sign
    }
    built_utc = [DateTime]::UtcNow.ToString("o")
    source_head = $head
    source_worktree_dirty = ($dirtyEntries.Count -gt 0)
    source_worktree_status = @($dirtyEntries)
    occt = [ordered]@{
        version = [string]$occtManifest.source.release
        source_commit = [string]$occtManifest.source.commit
        manifest_sha256 = Get-Sha256 $OcctManifestPath
        runtime_dll_count = @($occtManifest.shared_libraries).Count
    }
    files = $files
}
Write-Utf8 $packageManifestPath (($packageManifest | ConvertTo-Json -Depth 8) + "`n")

$issPath = Join-Path $OutputDir "manual-alpha.iss"
$escapedPayload = $payloadDir.Replace('"', '""')
$escapedOutput = $OutputDir.Replace('"', '""')
$iss = @"
[Setup]
AppId=KetchupManualAlpha
AppName=Ketchup Manual Alpha
AppVersion=$Version
AppPublisher=$publisher
DefaultDirName={localappdata}\Programs\Ketchup Manual Alpha
DefaultGroupName=Ketchup Manual Alpha
UninstallDisplayIcon={app}\Ketchup.exe
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
OutputDir=$escapedOutput
OutputBaseFilename=$setupBaseName
SetupLogging=yes
ChangesAssociations=yes

[Files]
Source: "$escapedPayload\*"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Ketchup Manual Alpha"; Filename: "{app}\Ketchup.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\Ketchup Manual Alpha"; Filename: "{app}\Ketchup.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Registry]
Root: HKA; Subkey: "Software\Classes\.ketchup"; ValueType: string; ValueName: ""; ValueData: "KetchupManualAlpha.Document"; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\KetchupManualAlpha.Document"; ValueType: string; ValueName: ""; ValueData: "Ketchup Document"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\KetchupManualAlpha.Document\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\Ketchup.exe,0"
Root: HKA; Subkey: "Software\Classes\KetchupManualAlpha.Document\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\Ketchup.exe"" ""%1"""

[Run]
Filename: "{app}\Ketchup.exe"; Description: "Launch Ketchup Manual Alpha"; WorkingDir: "{app}"; Flags: nowait postinstall skipifsilent
"@
Write-Utf8 $issPath ($iss.TrimStart() + "`r`n")

& $InnoCompilerPath $issPath
if ($LASTEXITCODE -ne 0) { throw "Inno Setup compilation failed." }
Assert-Leaf $setupPath "Manual Alpha installer"
if ($Sign) {
    Invoke-AzureSigning $setupPath
}
Remove-Item $issPath -Force

Compress-Archive -Path (Join-Path $payloadDir "*") -DestinationPath $zipPath -CompressionLevel Optimal
Assert-Leaf $zipPath "portable Manual Alpha archive"

$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("ketchup-manual-alpha-verify-" + [Guid]::NewGuid().ToString("N"))
$installedDir = Join-Path $tempRoot "installed"
try {
    [void](New-Item $tempRoot -ItemType Directory)
    $process = Start-Process -FilePath $setupPath -ArgumentList @(
        "/VERYSILENT",
        "/SUPPRESSMSGBOXES",
        "/NORESTART",
        "/NOICONS",
        "/DIR=$installedDir"
    ) -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Silent installer smoke test failed with exit code $($process.ExitCode)." }

    $manifest = Get-Content (Join-Path $installedDir "package-manifest.json") -Raw | ConvertFrom-Json
    if ($manifest.kind -cne "manual-alpha" -or
        $manifest.version -cne $Version -or
        $manifest.platform -cne "windows-x86_64" -or
        $manifest.assistant_visible -ne $false -or
        $manifest.code_signing.signed -ne [bool]$Sign -or
        $manifest.code_signing.timestamped -ne [bool]$Sign -or
        $manifest.occt.runtime_dll_count -ne @($occtManifest.shared_libraries).Count) {
        throw "Installed Manual Alpha manifest is invalid."
    }
    foreach ($record in @($manifest.files)) {
        $installedPath = Join-Path $installedDir ([string]$record.path)
        Assert-Leaf $installedPath "installed payload"
        if ((Get-Item $installedPath).Length -ne [int64]$record.size_bytes -or
            (Get-Sha256 $installedPath) -cne [string]$record.sha256) {
            throw "Installed payload fingerprint mismatch: $($record.path)"
        }
    }

    $workerResponse = ("PING" | & (Join-Path $installedDir "ketchup-exact-worker.exe")).Trim()
    if ($LASTEXITCODE -ne 0 -or $workerResponse -cne "PONG") {
        throw "Installed exact worker failed its headless PING."
    }

    $persistenceSmokePath = Join-Path $tempRoot "manual-alpha-persistence-smoke.ketchup"
    $manualAlphaVerification = (& (Join-Path $installedDir "Ketchup.exe") --verify-manual-alpha $persistenceSmokePath).Trim()
    $expectedVerification = "Ketchup Manual Alpha $Version verified; private-oauth codex-oauth gpt-5.6-sol"
    if ($LASTEXITCODE -ne 0 -or $manualAlphaVerification -cne $expectedVerification -or
        -not (Test-Path $persistenceSmokePath -PathType Leaf)) {
        throw "Installed application failed its headless Manual Alpha OAuth/startup/persistence check."
    }

    $uninstaller = Join-Path $installedDir "unins000.exe"
    Assert-Leaf $uninstaller "Manual Alpha uninstaller"
    $uninstall = Start-Process -FilePath $uninstaller -ArgumentList @(
        "/VERYSILENT",
        "/SUPPRESSMSGBOXES",
        "/NORESTART"
    ) -Wait -PassThru
    if ($uninstall.ExitCode -ne 0) { throw "Silent uninstall smoke test failed with exit code $($uninstall.ExitCode)." }
} finally {
    if (Test-Path $tempRoot) { Remove-Item $tempRoot -Recurse -Force }
}

$checksums = @(
    "$((Get-Sha256 $setupPath))  $([IO.Path]::GetFileName($setupPath))",
    "$((Get-Sha256 $zipPath))  $([IO.Path]::GetFileName($zipPath))",
    "$((Get-Sha256 $packageManifestPath))  payload/package-manifest.json"
)
Write-Utf8 (Join-Path $OutputDir "SHA256SUMS.txt") (($checksums -join "`r`n") + "`r`n")

$signingResult = if ($Sign) { "AEON-signed payload and installer, " } else { "" }
Write-Host "PASS: Manual Alpha ${signingResult}portable archive, exact payload hashes, worker PING, headless build identity, and uninstall smoke test."
Write-Host "Installer: $setupPath"
Write-Host "Portable:  $zipPath"
