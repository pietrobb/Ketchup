[CmdletBinding()]
param(
    # Output .apk path; an existing file is replaced.
    [string]$Output = "target\viewer-android\ketchup-viewer-debug.apk",
    # ABIs to include: arm64-v8a for phones, x86_64 for the emulator.
    [string[]]$Abi = @("arm64-v8a", "x86_64"),
    [switch]$Release,
    # Installs the APK on the single connected device/emulator and starts it,
    # with this .ketchup-view as the opened model.
    [string]$InstallWithModel,
    # Signing key. It lives outside target\ so `cargo clean` or another
    # checkout keeps the key the installed Viewer was signed with: Android
    # updates an app only with the same key, and uninstalling deletes the
    # saved session and earlier content kept after a cut-off save.
    [string]$Keystore = $(if ($env:KETCHUP_ANDROID_KEYSTORE) { $env:KETCHUP_ANDROID_KEYSTORE } else { Join-Path $env:USERPROFILE ".ketchup\viewer-android.keystore" })
)

# Builds the Android Kečup Viewer as an APK signed with a local debug key.
# Not for store publication: release signing is a separate, manual step.
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$sdk = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA "Android\Sdk" }
$ndk = Get-ChildItem (Join-Path $sdk "ndk") -Directory | Sort-Object { [version]$_.Name } | Select-Object -Last 1
$buildTools = Get-ChildItem (Join-Path $sdk "build-tools") -Directory | Sort-Object { [version]$_.Name } | Select-Object -Last 1
$platform = Get-ChildItem (Join-Path $sdk "platforms") -Directory | Sort-Object { [int]($_.Name -replace '\D') } | Select-Object -Last 1
$java = Get-Command keytool -ErrorAction SilentlyContinue
$keytool = if (Test-Path "C:\Program Files\Android\Android Studio\jbr\bin\keytool.exe") { "C:\Program Files\Android\Android Studio\jbr\bin\keytool.exe" } elseif ($java) { $java.Source } else { throw "keytool not found (install a JDK or Android Studio)" }
if (-not $ndk -or -not $buildTools -or -not $platform) { throw "Android SDK needs ndk, build-tools and platforms under $sdk" }
$env:JAVA_HOME = Split-Path (Split-Path $keytool)

$targets = @{ "arm64-v8a" = "aarch64-linux-android"; "x86_64" = "x86_64-linux-android" }
$buildProfile = if ($Release) { "release" } else { "debug" }
$targetDir = Join-Path $root "target\viewer-android"
$env:CARGO_TARGET_DIR = $targetDir
$clangDir = Join-Path $ndk.FullName "toolchains\llvm\prebuilt\windows-x86_64\bin"
$stage = Join-Path $targetDir "apk-stage"
Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
foreach ($abi in $Abi) {
    $triple = $targets[$abi]
    if (-not $triple) { throw "Unknown ABI $abi" }
    $var = $triple.ToUpper().Replace("-", "_")
    Set-Item "env:CARGO_TARGET_${var}_LINKER" (Join-Path $clangDir "$triple`28-clang.cmd")
    Set-Item "env:CC_$($triple.Replace('-', '_'))" (Join-Path $clangDir "$triple`28-clang.cmd")
    Set-Item "env:AR_$($triple.Replace('-', '_'))" (Join-Path $clangDir "llvm-ar.exe")
    & rustup target add $triple | Out-Null
    $cargoArgs = @("build", "-p", "ketchup-viewer-android", "--target", $triple, "--manifest-path", (Join-Path $root "Cargo.toml"))
    if ($Release) { $cargoArgs += "--release" }
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw "Rust build for $abi failed" }
    $libDir = New-Item (Join-Path $stage "lib\$abi") -ItemType Directory -Force
    # Debug info stays in target\; the APK carries stripped libraries.
    & (Join-Path $clangDir "llvm-strip.exe") --strip-debug -o (Join-Path $libDir "libketchup_viewer_android.so") (Join-Path $targetDir "$triple\$buildProfile\libketchup_viewer_android.so")
    if ($LASTEXITCODE -ne 0) { throw "llvm-strip for $abi failed" }
}

$androidJar = Join-Path $platform.FullName "android.jar"
# ViewerActivity (Java) carries the file traffic native code cannot do.
$classes = Join-Path $stage "classes"
New-Item $classes -ItemType Directory -Force | Out-Null
$sources = @((Get-ChildItem (Join-Path $root "viewer\android\java") -Recurse -Filter *.java).FullName)
& (Join-Path (Split-Path $keytool) "javac.exe") --release 11 -Xlint -Werror -classpath $androidJar -d $classes @sources
if ($LASTEXITCODE -ne 0) { throw "javac failed" }
$classFiles = @((Get-ChildItem $classes -Recurse -Filter *.class).FullName)
& (Join-Path $buildTools.FullName "d8.bat") --min-api 28 --lib $androidJar --output $stage @classFiles
if ($LASTEXITCODE -ne 0) { throw "d8 failed" }

$unsigned = Join-Path $targetDir "unsigned.apk"
$aligned = Join-Path $targetDir "aligned.apk"
# The manifest states no version: the APK gets the one in Cargo.toml.
. (Join-Path $PSScriptRoot "WorkspaceVersion.ps1")
$version = Get-WorkspaceVersion $root
$versionCode = Get-AndroidVersionCode $version
& (Join-Path $buildTools.FullName "aapt2.exe") link -o $unsigned -I $androidJar --manifest (Join-Path $root "viewer\android\AndroidManifest.xml") --version-name $version --version-code $versionCode
if ($LASTEXITCODE -ne 0) { throw "aapt2 link failed" }
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [System.IO.Compression.ZipFile]::Open($unsigned, "Update")
try {
    [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, (Join-Path $stage "classes.dex"), "classes.dex")
    foreach ($so in Get-ChildItem (Join-Path $stage "lib") -Recurse -File) {
        $entry = "lib/" + $so.Directory.Name + "/" + $so.Name
        [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, $so.FullName, $entry)
    }
} finally { $zip.Dispose() }
& (Join-Path $buildTools.FullName "zipalign.exe") -p -f 4 $unsigned $aligned
if ($LASTEXITCODE -ne 0) { throw "zipalign failed" }

$apksigner = Join-Path $buildTools.FullName "apksigner.bat"
# The SHA-256 digest of the certificate an APK is signed with.
function Get-SignerDigest([string]$Path) {
    $printed = & $apksigner verify --print-certs $Path
    if ($LASTEXITCODE -ne 0) { throw "apksigner could not read the signature of $Path" }
    $line = $printed | Select-String "certificate SHA-256 digest: ([0-9a-f]+)" | Select-Object -First 1
    if (-not $line) { throw "apksigner printed no certificate digest for $Path" }
    $line.Matches[0].Groups[1].Value
}

if (-not (Test-Path $Keystore)) {
    New-Item (Split-Path $Keystore) -ItemType Directory -Force | Out-Null
    # Builds before the key moved out of target\ signed with this one; the
    # Viewers they installed update only with it.
    $earlier = Join-Path $targetDir "debug.keystore"
    if (Test-Path $earlier) {
        Copy-Item $earlier $Keystore
        Write-Host "Signing key copied from $earlier to $Keystore"
    } else {
        & $keytool -genkeypair -keystore $Keystore -storepass android -keypass android -alias androiddebugkey -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=Android Debug,O=Android,C=US" | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "keytool failed" }
        Write-Host "New signing key $Keystore; keep it, Viewers signed with it update only with it"
    }
}
$apk = if ([System.IO.Path]::IsPathRooted($Output)) { $Output } else { Join-Path $root $Output }
& $apksigner sign --ks $Keystore --ks-pass pass:android --key-pass pass:android --out $apk $aligned
if ($LASTEXITCODE -ne 0) { throw "apksigner failed" }
$digest = Get-SignerDigest $apk
Write-Host "Android Viewer (debug-signed): $apk"
Write-Host "Signing certificate SHA-256: $digest ($Keystore)"

if ($InstallWithModel) {
    $adb = Join-Path $sdk "platform-tools\adb.exe"
    # A Viewer signed with another key cannot be updated, only uninstalled,
    # which deletes its saved session and kept earlier content; say so
    # before trying instead of leaving that as the way out.
    $installed = @(& $adb shell pm path sk.ketchup.viewer) | Where-Object { $_ -match '^package:(.+base\.apk)\s*$' } | ForEach-Object { $Matches[1] } | Select-Object -First 1
    if ($installed) {
        $pulled = Join-Path $targetDir "installed.apk"
        & $adb pull $installed $pulled | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "adb pull of the installed Viewer failed" }
        $installedDigest = Get-SignerDigest $pulled
        Remove-Item $pulled
        if ($installedDigest -ne $digest) {
            throw "The installed Viewer is signed with certificate SHA-256 $installedDigest, this APK with $digest from $Keystore. Android will not update it. Build again with -Keystore set to the key the installed Viewer was signed with; uninstalling it instead deletes its saved session and the earlier content it kept."
        }
    }
    & $adb install -r $apk
    if ($LASTEXITCODE -ne 0) { throw "adb install failed" }
    $remote = "/sdcard/Android/data/sk.ketchup.viewer/files/model.ketchup-view"
    & $adb shell mkdir -p (Split-Path $remote -Parent).Replace("\", "/")
    & $adb push $InstallWithModel $remote
    if ($LASTEXITCODE -ne 0) { throw "adb push failed" }
    & $adb shell am force-stop sk.ketchup.viewer
    & $adb shell am start -n sk.ketchup.viewer/.ViewerActivity
}
