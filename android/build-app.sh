#!/bin/sh
# Builds the FGS host APK by hand: the nsh runtime's classes.dex + the
# Rust app's libnsh.so + our manifest/Java (QuotaService). Same pipeline
# as nsh-cli's `package --target android`, plus a second dex and FGS
# permissions. No Gradle.
#
#   Prereqs: $ANDROID_HOME (SDK), ~/.nsh (runtime), a built libnsh.so
#   (`nsh package --target android --abi arm64-v8a`), a debug keystore.
#   Output: .nsh/out/mdrv-oq-fgs.apk
set -e

SDK=${ANDROID_HOME:-$HOME/Android/Sdk}
BT=$(ls -d "$SDK"/build-tools/* | sort | tail -1)
PLATFORM="$SDK/platforms/android-35/android.jar"
RUNTIME="$HOME/.nsh/runtime/android/release"
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LIB="$ROOT/.nsh/rust/android-rust-release/lib/arm64-v8a/libnsh.so"
OUT="$ROOT/android/build"
KEYSTORE=${MDRV_OQ_KEYSTORE:-$HOME/.android/debug.keystore}
KEYPASS=${NSH_KEYSTORE_PASSWORD:-android}
APK="$ROOT/.nsh/out/mdrv-oq-fgs.apk"

[ -f "$LIB" ] || { echo "no libnsh.so at $LIB — run nsh package first" >&2; exit 1; }
[ -f "$KEYSTORE" ] || {
    mkdir -p "$HOME/.android"
    keytool -genkeypair -keystore "$KEYSTORE" -alias androiddebugkey \
        -storepass android -keypass android -keyalg RSA -keysize 2048 \
        -validity 10000 -dname "CN=Android Debug,O=Android,C=US" >/dev/null
}

rm -rf "$OUT"
mkdir -p "$OUT/classes" "$OUT/lib/arm64-v8a"

# 1. Our Java -> dex (classes2.dex; the runtime's is classes.dex).
javac --release 11 -classpath "$PLATFORM" -d "$OUT/classes" \
    $(find "$ROOT"/android/src -name '*.java')
"$BT/d8" --release --lib "$PLATFORM" --min-api 26 --output "$OUT" \
    $(find "$OUT/classes/id" -name '*.class')
mv "$OUT/classes.dex" "$OUT/classes2.dex"

# 2. Resources (adaptive launcher icon), manifest + assets.
"$BT/aapt2" compile --dir "$ROOT/android/res" -o "$OUT/res.zip"
"$BT/aapt2" link -o "$OUT/base.apk" -I "$PLATFORM" \
    --manifest "$ROOT/android/AndroidManifest.xml" \
    "$OUT/res.zip" \
    -A "$ROOT/android/assets"

# 3. dex + the Rust lib (stored, page-aligned: extractNativeLibs=false).
cp "$RUNTIME/classes.dex" "$OUT/classes.dex"
(cd "$OUT" && zip -q base.apk classes.dex classes2.dex)
cp "$LIB" "$OUT/lib/arm64-v8a/libnsh.so"
(cd "$OUT" && zip -q -0 base.apk lib/arm64-v8a/libnsh.so)

# 4. Align, sign.
"$BT/zipalign" -f -p 16384 "$OUT/base.apk" "$OUT/aligned.apk"
"$BT/apksigner" sign \
    --ks "$KEYSTORE" --ks-key-alias androiddebugkey --ks-pass "pass:$KEYPASS" \
    --out "$APK" "$OUT/aligned.apk"

echo "built $APK ($(du -h "$APK" | cut -f1))"
