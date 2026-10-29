[CmdletBinding()]
param(
    # Defaults to [workspace.package] version in Cargo.toml; any other is refused.
    [string]$Version,
    [string]$OutputDir,
    [string]$BinaryDir,
    [string]$OcctRoot,
    [string]$OcctManifestPath,
    [string]$InnoCompilerPath,
    [string]$SigningMetadataPath,
    [string]$SignToolPath,
    [string]$AzureSigningDlibPath,
    # A real .ketchup-view exported by Kecup; the installed Viewer must open it.
    [string]$ViewerSmokePackage,
    # Folder with the examples README.md links to a release download (tiny-house.ketchup);
    # with it the release also gets Ketchup-<version>-Examples.zip (build-examples-zip.ps1).
    [string]$DownloadedExamples,
    [switch]$Sign,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
. (Join-Path $PSScriptRoot "WorkspaceVersion.ps1")
$workspaceVersion = Get-WorkspaceVersion $repoRoot
if ([string]::IsNullOrWhiteSpace($Version)) {
    $Version = $workspaceVersion
} elseif ($Version -cne $workspaceVersion) {
    throw "-Version $Version is not $workspaceVersion, the version in Cargo.toml that the programs report; change [workspace.package] version there instead."
}
if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path $repoRoot "artifacts\release\$Version"
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
$setupBaseName = "Ketchup-$Version-Setup"
$setupPath = Join-Path $OutputDir "$setupBaseName.exe"
$zipPath = Join-Path $OutputDir "Ketchup-$Version-Portable.zip"
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
    throw "Installer packaging is Windows x64 only."
}
if ($Version -notmatch '^[0-9A-Za-z][0-9A-Za-z.-]{0,63}$') {
    throw "Version contains unsupported characters: $Version"
}
Assert-Leaf $OcctManifestPath "pinned OCCT manifest"
Assert-Leaf $InnoCompilerPath "Inno Setup compiler"
if (-not [string]::IsNullOrWhiteSpace($ViewerSmokePackage)) {
    $ViewerSmokePackage = [IO.Path]::GetFullPath($ViewerSmokePackage)
    Assert-Leaf $ViewerSmokePackage "Viewer smoke package"
}
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
        throw "OutputDir must be absent or empty; refusing to overwrite release artifacts: $OutputDir"
    }
} else {
    [void](New-Item $OutputDir -ItemType Directory)
}
[void](New-Item $payloadDir -ItemType Directory)
# Before the build, so a missing download fails at once.
$examplesPath = Join-Path $OutputDir "Ketchup-$Version-Examples.zip"
if ([string]::IsNullOrWhiteSpace($DownloadedExamples)) {
    Write-Warning "Examples zip skipped: pass -DownloadedExamples with the files README.md links to a release download."
} else {
    & (Join-Path $PSScriptRoot "build-examples-zip.ps1") -OutputDir $OutputDir -DownloadedExamples $DownloadedExamples -Version $Version
    Assert-Leaf $examplesPath "examples archive"
}

if (-not $SkipBuild) {
    Push-Location $repoRoot
    try {
        $previousBuildVersion = [Environment]::GetEnvironmentVariable("KETCHUP_BUILD_VERSION", "Process")
        try {
            [Environment]::SetEnvironmentVariable("KETCHUP_BUILD_VERSION", $Version, "Process")
            & cargo build --locked --release -p ketchup-app --bin ketchup-app
            if ($LASTEXITCODE -ne 0) { throw "Application release build failed." }
        } finally {
            [Environment]::SetEnvironmentVariable("KETCHUP_BUILD_VERSION", $previousBuildVersion, "Process")
        }
        & cargo build --locked --release -p ketchup-scheduler --bin ketchup-exact-worker --bin ketchup-program-worker
        if ($LASTEXITCODE -ne 0) { throw "Worker release build failed." }
        & cargo build --locked --release -p ketchup-viewer --bin ketchup-viewer
        if ($LASTEXITCODE -ne 0) { throw "Viewer release build failed." }
    } finally {
        Pop-Location
    }
}

$appSource = Join-Path $BinaryDir "ketchup-app.exe"
$workerSource = Join-Path $BinaryDir "ketchup-exact-worker.exe"
Assert-Leaf $appSource "application"
Assert-Leaf $workerSource "exact worker"
$programWorkerSource = Join-Path $BinaryDir "ketchup-program-worker.exe"
Assert-Leaf $programWorkerSource "program worker"
$viewerSource = Join-Path $BinaryDir "ketchup-viewer.exe"
Assert-Leaf $viewerSource "Viewer"
Add-Type -Namespace KetchupBuild -Name Shell -MemberDefinition @'
[DllImport("shell32.dll", CharSet = CharSet.Unicode)]
public static extern uint ExtractIconEx(string file, int index, IntPtr[] large, IntPtr[] small, uint count);
'@
foreach ($iconSource in @($appSource, $viewerSource)) {
    if ([KetchupBuild.Shell]::ExtractIconEx($iconSource, -1, $null, $null, 0) -lt 1) {
        throw "Executable has no embedded icon: $iconSource"
    }
}
Copy-Item $appSource (Join-Path $payloadDir "Ketchup.exe")
Copy-Item $workerSource (Join-Path $payloadDir "ketchup-exact-worker.exe")
Copy-Item $programWorkerSource (Join-Path $payloadDir "ketchup-program-worker.exe")
Copy-Item $viewerSource (Join-Path $payloadDir "KetchupViewer.exe")

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
. (Join-Path $PSScriptRoot "package-notices.ps1")
Copy-PackageNotices $repoRoot $OcctRoot $payloadDir
if ($Sign) {
    Invoke-AzureSigning (Join-Path $payloadDir "Ketchup.exe")
    Invoke-AzureSigning (Join-Path $payloadDir "ketchup-exact-worker.exe")
    Invoke-AzureSigning (Join-Path $payloadDir "ketchup-program-worker.exe")
    Invoke-AzureSigning (Join-Path $payloadDir "KetchupViewer.exe")
}
$publisher = if ($Sign) { "AEON GROUP spol. s r.o." } else { "Ketchup" }
$distributionNotice = if ($Sign) {
    "Program je digitalne podpisany vydavatelom AEON GROUP spol. s r.o."
} else {
    "Nepodpisany build. Windows SmartScreen moze pri prvom spusteni zobrazit upozornenie."
}

$startHere = @"
KECUP $Version
==============

Kecup je 2D/3D parametricky modelar. Nie je potrebny ucet, API kluc ani Python.
AI klienta s podporou MCP (Claude Code, Codex) pripojite v pravom paneli v casti "Pripojit AI".

ODPORUCANY PRVY POKUS (priblizne 30-60 minut)
1. Spustite Ketchup a vytvorte prazdny dokument.
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
10. Subor > Exportovat pre Viewer vytvori .ketchup-view. Dvojklik nanho ho otvori v programe
    Ketchup Viewer, kde mozno pridat pripomienky a ulozit .ketchup-comments. Dvojklik na
    .ketchup-comments ho otvori vo Vieweri spolu s balikom .ketchup-view v tom istom priecinku.
11. DWG: Subor > Importovat ohranicenu 2D podmnozinu DXF/DWG nacita aj vykres .dwg (prevedie
    ho do DXF a ukaze, co sa nedalo precitat). Subor > Vykresy (PDF, DXF, DWG) zapise vykresovy
    list dokumentu ako DXF alebo DWG (AutoCAD 2018); kruhy a obluky idu ako skutocne kruznice
    a obluky, skryte hrany vo vlastnej hladine. Vykresovy list vytvorite cez Nahlad vykresu
    z vyberu v sekcii ZOSTAVA.

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
KECUP $Version - SPATNA VAZBA
==========================

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
Write-Utf8 (Join-Path $payloadDir "VERSION.txt") ("Ketchup $Version`r`nWindows x64`r`n")

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
    kind = "release"
    version = $Version
    platform = "windows-x86_64"
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
    viewer = [ordered]@{
        executable = "KetchupViewer.exe"
        file_types = @(".ketchup-view", ".ketchup-comments")
    }
    files = $files
}
Write-Utf8 $packageManifestPath (($packageManifest | ConvertTo-Json -Depth 8) + "`n")

$issPath = Join-Path $OutputDir "ketchup.iss"
$escapedPayload = $payloadDir.Replace('"', '""')
$escapedOutput = $OutputDir.Replace('"', '""')
$setupIcon = Join-Path $repoRoot "crates\ketchup-app\assets\ketchup.ico"
Assert-Leaf $setupIcon "application icon"
$escapedSetupIcon = $setupIcon.Replace('"', '""')
# Inno signs the uninstaller it embeds only through a SignTool given to ISCC.
$signedUninstaller = if ($Sign) { "SignTool=ketchupsign`r`nSignedUninstaller=yes" } else { "" }
# The AppId stays fixed for every release: changing it installs side by side instead of upgrading.
$iss = @"
[Setup]
AppId={{C73827F1-5B7A-42CB-8D5E-8B40441C3EBD}
AppName=Ketchup
AppVersion=$Version
AppPublisher=$publisher
DefaultDirName={localappdata}\Programs\Ketchup
DefaultGroupName=Ketchup
UninstallDisplayIcon={app}\Ketchup.exe
SetupIconFile=$escapedSetupIcon
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
$signedUninstaller

[Files]
Source: "$escapedPayload\*"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Ketchup"; Filename: "{app}\Ketchup.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\Ketchup"; Filename: "{app}\Ketchup.exe"; WorkingDir: "{app}"; Tasks: desktopicon
Name: "{group}\Ketchup Viewer"; Filename: "{app}\KetchupViewer.exe"; WorkingDir: "{app}"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Registry]
Root: HKA; Subkey: "Software\Classes\.ketchup"; ValueType: string; ValueName: ""; ValueData: "Ketchup.Document"; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\Ketchup.Document"; ValueType: string; ValueName: ""; ValueData: "Ketchup Document"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Ketchup.Document\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\Ketchup.exe,0"
Root: HKA; Subkey: "Software\Classes\Ketchup.Document\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\Ketchup.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\.ketchup-view"; ValueType: string; ValueName: ""; ValueData: "Ketchup.ViewerPackage"; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\Ketchup.ViewerPackage"; ValueType: string; ValueName: ""; ValueData: "Ketchup Viewer Package"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Ketchup.ViewerPackage\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\KetchupViewer.exe,0"
Root: HKA; Subkey: "Software\Classes\Ketchup.ViewerPackage\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\KetchupViewer.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\.ketchup-comments"; ValueType: string; ValueName: ""; ValueData: "Ketchup.ViewerComments"; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\Ketchup.ViewerComments"; ValueType: string; ValueName: ""; ValueData: "Ketchup Viewer Comments"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Ketchup.ViewerComments\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\KetchupViewer.exe,0"
Root: HKA; Subkey: "Software\Classes\Ketchup.ViewerComments\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\KetchupViewer.exe"" ""%1"""

[Run]
Filename: "{app}\Ketchup.exe"; Parameters: "--mcp-setup"; Description: "Connect Claude Code and Codex to Ketchup (if installed)"; WorkingDir: "{app}"; Flags: postinstall runhidden skipifsilent
Filename: "{app}\Ketchup.exe"; Description: "Launch Ketchup"; WorkingDir: "{app}"; Flags: nowait postinstall skipifsilent

[Code]
// Test builds before 0.1.0 installed under another AppId. Remove them first, so the
// release takes over instead of sharing .ketchup with an uninstaller that later deletes it.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Uninstaller: String;
  ResultCode: Integer;
begin
  Result := '';
  if RegQueryStringValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Uninstall\KetchupManualAlpha_is1', 'UninstallString', Uninstaller) then
    Exec(RemoveQuotes(Uninstaller), '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;
"@
Write-Utf8 $issPath ($iss.TrimStart() + "`r`n")

$isccArguments = @($issPath)
if ($Sign) {
    $isccArguments = @(
        "/Sketchupsign=`$q$SignToolPath`$q sign /v /fd SHA256 /tr http://timestamp.acs.microsoft.com /td SHA256 /dlib `$q$AzureSigningDlibPath`$q /dmdf `$q$SigningMetadataPath`$q `$f",
        $issPath
    )
}
& $InnoCompilerPath @isccArguments
if ($LASTEXITCODE -ne 0) { throw "Inno Setup compilation failed." }
Assert-Leaf $setupPath "installer"
if ($Sign) {
    Invoke-AzureSigning $setupPath
}
Remove-Item $issPath -Force

Compress-Archive -Path (Join-Path $payloadDir "*") -DestinationPath $zipPath -CompressionLevel Optimal
Assert-Leaf $zipPath "portable archive"
Assert-PackageNotices $zipPath "The portable archive"

$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("ketchup-installer-verify-" + [Guid]::NewGuid().ToString("N"))
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
    if ($manifest.kind -cne "release" -or
        $manifest.version -cne $Version -or
        $manifest.platform -cne "windows-x86_64" -or
        $manifest.code_signing.signed -ne [bool]$Sign -or
        $manifest.code_signing.timestamped -ne [bool]$Sign -or
        $manifest.occt.runtime_dll_count -ne @($occtManifest.shared_libraries).Count) {
        throw "Installed package manifest is invalid."
    }
    foreach ($record in @($manifest.files)) {
        $installedPath = Join-Path $installedDir ([string]$record.path)
        Assert-Leaf $installedPath "installed payload"
        if ((Get-Item $installedPath).Length -ne [int64]$record.size_bytes -or
            (Get-Sha256 $installedPath) -cne [string]$record.sha256) {
            throw "Installed payload fingerprint mismatch: $($record.path)"
        }
    }
    Assert-PackageNotices $installedDir "The installed program"

    . (Join-Path $PSScriptRoot "exact-worker-handshake.ps1")
    Assert-ExactWorkerHandshake (Join-Path $installedDir "ketchup-exact-worker.exe") "Installed exact worker"
    Assert-ExactWorkerHandshake (Join-Path $installedDir "ketchup-program-worker.exe") "Installed program worker"

    $persistenceSmokePath = Join-Path $tempRoot "install-persistence-smoke.ketchup"
    # Ketchup.exe is a windowed app: only a pipeline makes PowerShell wait for its output.
    $installVerification = (& (Join-Path $installedDir "Ketchup.exe") --verify-install $persistenceSmokePath | Out-String).Trim()
    $expectedVerification = "Ketchup $Version verified"
    if ($LASTEXITCODE -ne 0 -or $installVerification -cne $expectedVerification -or
        -not (Test-Path $persistenceSmokePath -PathType Leaf)) {
        throw "Installed application failed its headless startup/persistence check."
    }

    # The system opens documents with the installed Ketchup and both Viewer file types with the installed Viewer.
    $installedViewer = Join-Path $installedDir "KetchupViewer.exe"
    $expectedAppCommand = """$(Join-Path $installedDir "Ketchup.exe")"" ""%1"""
    $expectedViewerCommand = """$installedViewer"" ""%1"""
    foreach ($association in @(
            @{ extension = ".ketchup"; progId = "Ketchup.Document"; command = $expectedAppCommand },
            @{ extension = ".ketchup-view"; progId = "Ketchup.ViewerPackage"; command = $expectedViewerCommand },
            @{ extension = ".ketchup-comments"; progId = "Ketchup.ViewerComments"; command = $expectedViewerCommand })) {
        $registered = (Get-ItemProperty "HKCU:\Software\Classes\$($association.extension)" -ErrorAction SilentlyContinue).'(default)'
        $command = (Get-ItemProperty "HKCU:\Software\Classes\$($association.progId)\shell\open\command" -ErrorAction SilentlyContinue).'(default)'
        if ($registered -cne $association.progId -or $command -ne $association.command) {
            throw "Installed package is not registered for $($association.extension)."
        }
    }
    if ([string]::IsNullOrWhiteSpace($ViewerSmokePackage)) {
        Write-Warning "Viewer open smoke skipped: pass -ViewerSmokePackage with a real exported .ketchup-view."
    } else {
        $viewerVerification = (& $installedViewer --verify-open $ViewerSmokePackage | Out-String).Trim()
        if ($LASTEXITCODE -ne 0 -or $viewerVerification -notmatch '^Ketchup Viewer \S+ opened [1-9][0-9]* components, [0-9]+ scenes, [0-9]+ comments$') {
            throw "Installed Viewer could not open the smoke package: $viewerVerification"
        }
        Write-Host $viewerVerification
    }

    $uninstaller = Join-Path $installedDir "unins000.exe"
    Assert-Leaf $uninstaller "uninstaller"
    if ($Sign) {
        $uninstallerSignature = Get-AuthenticodeSignature $uninstaller
        if ($uninstallerSignature.Status -ne [Management.Automation.SignatureStatus]::Valid -or
            $uninstallerSignature.SignerCertificate.Subject -notlike 'CN=AEON GROUP spol. s r.o.*') {
            throw "The installed uninstaller is not signed by AEON GROUP."
        }
    }
    $uninstall = Start-Process -FilePath $uninstaller -ArgumentList @(
        "/VERYSILENT",
        "/SUPPRESSMSGBOXES",
        "/NORESTART"
    ) -Wait -PassThru
    if ($uninstall.ExitCode -ne 0) { throw "Silent uninstall smoke test failed with exit code $($uninstall.ExitCode)." }
    foreach ($key in @("Ketchup.Document", "Ketchup.ViewerPackage", "Ketchup.ViewerComments")) {
        if (Test-Path "HKCU:\Software\Classes\$key") { throw "Uninstall left the association $key." }
    }
    if (Test-Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{C73827F1-5B7A-42CB-8D5E-8B40441C3EBD}_is1") {
        throw "Uninstall left its Add/Remove Programs entry."
    }
} finally {
    if (Test-Path $tempRoot) { Remove-Item $tempRoot -Recurse -Force }
}

$checksums = @(
    "$((Get-Sha256 $setupPath))  $([IO.Path]::GetFileName($setupPath))",
    "$((Get-Sha256 $zipPath))  $([IO.Path]::GetFileName($zipPath))",
    "$((Get-Sha256 $packageManifestPath))  payload/package-manifest.json"
)
if (-not [string]::IsNullOrWhiteSpace($DownloadedExamples)) {
    $checksums += "$((Get-Sha256 $examplesPath))  $([IO.Path]::GetFileName($examplesPath))"
}
Write-Utf8 (Join-Path $OutputDir "SHA256SUMS.txt") (($checksums -join "`r`n") + "`r`n")

$signingResult = if ($Sign) { "AEON-signed payload and installer, " } else { "" }
Write-Host "PASS: ${signingResult}portable archive, third-party notices, exact payload hashes, worker handshake, headless build identity, document and Viewer file associations, and uninstall smoke test."
Write-Host "Installer: $setupPath"
Write-Host "Portable:  $zipPath"
if (-not [string]::IsNullOrWhiteSpace($DownloadedExamples)) { Write-Host "Examples:  $examplesPath" }
