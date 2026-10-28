#!/usr/bin/env bash
# Construit Folie-DMX.app (Universal : Apple Silicon et Intel), la signe avec le certificat
# Developer ID, produit le DMG et le notarise.
#
#   scripts/package-macos.sh
#
# Variables :
#   SIGN_IDENTITY    identité Developer ID (défaut : la seule « Developer ID Application » du trousseau)
#   NOTARY_PROFILE   profil `xcrun notarytool store-credentials` ; sans lui, le DMG est signé
#                    mais pas notarisé (macOS avertira à l'ouverture)
#   OUT_DIR          où poser le DMG (défaut : dist/ du dépôt)

set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
out_dir=${OUT_DIR:-"$repo/dist"}
bundle_id=com.michaelmambole.folie-dmx

step() { printf '\n\033[1m== %s\033[0m\n' "$*"; }
fail() { printf '\033[31mÉchec : %s\033[0m\n' "$*" >&2; exit 1; }

identity=${SIGN_IDENTITY:-$(security find-identity -v -p codesigning \
    | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | head -1)}
[ -n "$identity" ] || fail "aucune identité Developer ID Application dans le trousseau"

cd "$repo"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
commit=$(git rev-parse --short HEAD)
[ -z "$(git status --porcelain -- crates library Cargo.toml Cargo.lock)" ] \
    || echo "Attention : modifications non commitées, le paquet ne correspond pas à $commit."

work=$(mktemp -d "${TMPDIR:-/tmp}/package-macos.XXXXXX")
trap 'rm -rf "$work"' EXIT

# --- Compilation Universal --------------------------------------------------
step "Compilation (Apple Silicon et Intel)"
for target in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --release --locked --bin folie-dmx --bin folie --target "$target"
done
for bin in folie-dmx folie; do
    lipo -create -output "$work/$bin" \
        "target/aarch64-apple-darwin/release/$bin" "target/x86_64-apple-darwin/release/$bin"
done

# --- Icône --------------------------------------------------------------------
step "Icône"
FOLIE_ICONSET="$work/Folie.iconset" cargo test -q -p folie-app --locked macos_iconset >/dev/null
iconutil -c icns "$work/Folie.iconset" -o "$work/Folie.icns"

# --- Bundle --------------------------------------------------------------------
step "Folie-DMX.app $version ($commit)"
app="$work/Folie-DMX.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$work/folie-dmx" "$work/folie" "$app/Contents/MacOS/"
cp "$work/Folie.icns" "$app/Contents/Resources/"
cp -R library "$app/Contents/Resources/library"
cp LICENSE "$app/Contents/Resources/"
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key><string>$bundle_id</string>
    <key>CFBundleName</key><string>Folie-DMX</string>
    <key>CFBundleDisplayName</key><string>Folie-DMX</string>
    <key>CFBundleExecutable</key><string>folie-dmx</string>
    <key>CFBundleIconFile</key><string>Folie</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version.$commit</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.music</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSHumanReadableCopyright</key><string>Michaël Mambole — GPL-3.0-or-later</string>
    <key>NSMicrophoneUsageDescription</key><string>Folie-DMX écoute l'entrée son choisie pour faire réagir la lumière à la musique.</string>
    <key>NSLocalNetworkUsageDescription</key><string>Folie-DMX envoie l'Art-Net et le sACN aux projecteurs, et reçoit la télécommande et l'OSC.</string>
</dict>
</plist>
EOF
cat > "$work/entitlements.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.device.audio-input</key>
    <true/>
</dict>
</plist>
EOF
plutil -lint "$app/Contents/Info.plist" >/dev/null

# --- Signature : de l'intérieur vers l'extérieur, runtime renforcé (exigé pour la notarisation)
step "Signature ($identity)"
codesign --force --timestamp --options runtime --sign "$identity" "$app/Contents/MacOS/folie"
codesign --force --timestamp --options runtime --entitlements "$work/entitlements.plist" \
    --sign "$identity" "$app"
codesign --verify --strict --deep --verbose=2 "$app"

# --- DMG -----------------------------------------------------------------------
step "DMG"
staging="$work/dmg"
mkdir "$staging"
ditto "$app" "$staging/Folie-DMX.app"
ln -s /Applications "$staging/Applications"
mkdir -p "$out_dir"
out_dir=$(cd "$out_dir" && pwd)
dmg="$out_dir/Folie-DMX-$version-$commit-macOS-universal.dmg"
rm -f "$dmg"
hdiutil create -quiet -volname "Folie-DMX $version" -srcfolder "$staging" \
    -fs HFS+ -format UDZO -imagekey zlib-level=9 "$dmg"
codesign --force --timestamp --sign "$identity" "$dmg"

if [ -n "${NOTARY_PROFILE:-}" ]; then
    step "Notarisation (quelques minutes)"
    xcrun notarytool submit "$dmg" --keychain-profile "$NOTARY_PROFILE" --wait | tee "$work/notary.log"
    grep -q "status: Accepted" "$work/notary.log" || fail "notarisation refusée (xcrun notarytool log <id> --keychain-profile $NOTARY_PROFILE)"
    xcrun stapler staple "$dmg"
    spctl --assess --type open --context context:primary-signature -vv "$dmg"
else
    echo "NOTARY_PROFILE absent : DMG signé mais non notarisé."
fi

step "Terminé"
echo "  $dmg"
echo "  SHA-256 $(shasum -a 256 "$dmg" | cut -d' ' -f1)"
