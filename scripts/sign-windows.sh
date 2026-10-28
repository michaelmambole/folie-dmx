#!/usr/bin/env bash
# Signe les .exe d'un paquet Windows avec le certificat SSL.com eSigner, depuis le Mac,
# puis produit le zip de distribution vérifié.
#
#   scripts/sign-windows.sh                 dernier passage réussi de la compilation Windows (GitHub)
#   scripts/sign-windows.sh <run-id>        un passage précis
#   scripts/sign-windows.sh <dossier>       un paquet déjà là (MikaDAW…) : tous ses .exe sont signés
#
# Variables :
#   SSL_CRED_FILE    identifiants SSL.com (défaut : sslcred.md de MikaDAW), lignes « libellé : valeur »
#                    lues par libellé : « login », « mot de passe », et « secret key » (secret OTP)
#                    s'il y est. Jamais affichés, jamais copiés ailleurs.
#   CODESIGN_TOOL    dossier de CodeSignTool (défaut : ~/Tools/CodeSignTool-1.3.2)
#   JAVA             java à utiliser (défaut : openjdk de Homebrew)
#   OUT_DIR          où poser le zip signé (défaut : dist/ du dépôt)
#   ZIP_NAME         nom du zip (défaut : <paquet>-<version>-Windows-x64.zip)
#   CHECK_ONLY=1     s'arrête après la lecture du certificat : rien n'est analysé ni signé
#
# L'OTP eSigner : avec le secret OTP (« secret key » du fichier, ou à défaut le trousseau du Mac,
# service « ssl-esigner-totp »), CodeSignTool calcule le code lui-même. Sinon il se tape dans le
# terminal quand il est demandé : un seul pour tous les .exe.

set -euo pipefail
set +x

repo=$(cd "$(dirname "$0")/.." && pwd)
cred_file=${SSL_CRED_FILE:-"$HOME/Documents/plan d'implantation/mini-daw-pedagogique/sslcred.md"}
tool=${CODESIGN_TOOL:-"$HOME/Tools/CodeSignTool-1.3.2"}
java=${JAVA:-/opt/homebrew/opt/openjdk/bin/java}
out_dir=${OUT_DIR:-"$repo/dist"}
expected_signer="CN=Michael Mambole"

step() { printf '\n\033[1m== %s\033[0m\n' "$*"; }
fail() { printf '\033[31mÉchec : %s\033[0m\n' "$*" >&2; exit 1; }

for cmd in osslsigncode shasum zip unzip; do
    command -v "$cmd" >/dev/null || fail "$cmd introuvable"
done
[ -x "$java" ] || fail "java introuvable ($java)"
jar=$(ls "$tool"/jar/code_sign_tool-*.jar 2>/dev/null | head -1)
[ -n "$jar" ] || fail "CodeSignTool introuvable dans $tool"
[ -r "$cred_file" ] || fail "identifiants introuvables ($cred_file)"

# --- Identifiants (jamais affichés) ----------------------------------------
username="" password="" secret=""
while IFS= read -r line || [ -n "$line" ]; do
    case $line in *:*) ;; *) continue ;; esac
    label=$(printf '%s' "${line%%:*}" | tr '[:upper:]' '[:lower:]' | sed 's/^ *//; s/ *$//')
    value=${line#*:}
    value=${value# }
    case $label in
        login) username=$value ;;
        "mot de passe") password=$value ;;
        "secret key") secret=$value ;;
    esac
done < "$cred_file"
unset line value label
[ -n "$username" ] && [ -n "$password" ] \
    || fail "« login » ou « mot de passe » absent de $(basename "$cred_file")"
if [ -z "$secret" ]; then
    secret=$(security find-generic-password -s ssl-esigner-totp -w 2>/dev/null || true)
fi
# Le secret d'une app OTP est en base32 ; CodeSignTool l'attend en base64 (format affiché par SSL.com).
if [[ $secret =~ ^[A-Z2-7]+=*$ ]]; then
    secret=$(python3 -c 'import base64,sys; s=sys.stdin.read().strip(); print(base64.b64encode(base64.b32decode(s + "=" * (-len(s) % 8))).decode())' <<<"$secret")
fi
totp=()
[ -z "$secret" ] || totp=("-totp_secret=$secret")
unset secret
[ -n "${CHECK_ONLY:-}" ] || [ ${#totp[@]} -gt 0 ] || [ -t 0 ] || fail "à lancer dans un terminal : l'OTP se tape au clavier"

work=$(mktemp -d "${TMPDIR:-/tmp}/sign-windows.XXXXXX")
trap 'rm -rf "$work"' EXIT

# --- Paquet à signer --------------------------------------------------------
arg=${1:-}
if [ -n "$arg" ] && [ -d "$arg" ]; then
    package=$(cd "$arg" && pwd)
    step "Paquet local : $package"
    cp -R "$package" "$work/"
    package="$work/$(basename "$package")"
    version=""
else
    command -v gh >/dev/null || fail "gh introuvable"
    cd "$repo"
    run=${arg:-$(gh run list --workflow windows.yml --status success --limit 1 --json databaseId -q '.[0].databaseId')}
    [ -n "$run" ] || fail "aucune compilation Windows réussie"
    commit=$(gh run view "$run" --json headSha -q .headSha)
    step "Compilation GitHub $run (commit ${commit:0:7})"
    remote_head=$(git rev-parse origin/master 2>/dev/null || true)
    if [ "$commit" != "$remote_head" ]; then
        echo "Attention : ce n'est pas le dernier commit poussé (${remote_head:0:7})."
        read -r -p "Continuer quand même ? [o/N] " answer
        [ "$answer" = o ] || exit 1
    fi
    gh run download "$run" --name Folie-DMX-windows-NON-SIGNE --dir "$work/Folie-DMX"
    package="$work/Folie-DMX"
    version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)-${commit:0:7}"
fi

exes=()
while IFS= read -r f; do exes+=("$f"); done < <(find "$package" -maxdepth 1 -iname '*.exe' | sort)
[ ${#exes[@]} -gt 0 ] || fail "aucun .exe dans le paquet"
for exe in "${exes[@]}"; do
    if osslsigncode verify -in "$exe" >/dev/null 2>&1; then
        fail "$(basename "$exe") est déjà signé"
    fi
    printf '  %s  %s\n' "$(shasum -a 256 "$exe" | cut -c1-16)…" "$(basename "$exe")"
done

# CodeSignTool sort avec 0 même en cas d'erreur : on lit sa sortie.
cst() {
    local log
    log=$(mktemp "$work/cst.XXXXXX")
    "$java" -jar "$jar" "$@" 2>&1 | grep -v "getCallerClass is not supported" | tee "$log"
    ! grep -q "^Error" "$log"
}
export CODE_SIGN_TOOL_PATH="$tool"
cd "$tool"   # CodeSignTool lit conf/ dans le dossier courant

step "Certificat"
ids=$(cst get_credential_ids "-username=$username" "-password=$password" 2>&1) \
    || fail "get_credential_ids : $(grep -i error <<<"$ids" | head -1)"
credential=$(grep -o -E '[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}' <<<"$ids" | head -1)
[ -n "$credential" ] || fail "aucun identifiant de certificat retourné"
echo "  ${credential:0:8}…"
if [ ${#totp[@]} -gt 0 ]; then echo "  secret OTP trouvé : signature sans OTP à taper"; else echo "  OTP à taper (pas de secret OTP)"; fi
if [ -n "${CHECK_ONLY:-}" ]; then
    step "Vérification seule : tout est prêt, rien n'a été signé"
    exit 0
fi

# --- Analyse puis signature groupée ----------------------------------------
input="$work/sign-input" output="$work/signed"
mkdir -p "$input" "$output"
for exe in "${exes[@]}"; do cp "$exe" "$input/"; done

for exe in "$input"/*; do
    step "Analyse de $(basename "$exe")"
    cst scan_code "-credential_id=$credential" "-username=$username" "-password=$password" \
        "-input_file_path=$exe" || fail "scan_code $(basename "$exe")"
done

if [ ${#totp[@]} -gt 0 ]; then
    step "Signature (OTP calculé à partir du secret)"
else
    step "Signature (tapez l'OTP eSigner quand il est demandé)"
fi
sign_args=("-credential_id=$credential" "-username=$username" "-password=$password"
    ${totp[@]+"${totp[@]}"} "-input_dir_path=$input" "-output_dir_path=$output")
if [ ${#totp[@]} -gt 0 ]; then
    cst batch_sign "${sign_args[@]}" || fail "batch_sign"
else
    # Invite « Enter the OTP » sans retour à la ligne : sortie directe, contrôle par les fichiers signés.
    "$java" -jar "$jar" batch_sign "${sign_args[@]}"
fi
unset sign_args
unset password username totp

# --- Vérifications ----------------------------------------------------------
verify() {
    local exe=$1 report
    report=$(osslsigncode verify -in "$exe" 2>&1) || { echo "$report" >&2; fail "signature invalide : $exe"; }
    grep -q "Signature verification: ok" <<<"$report" || fail "signature non vérifiée : $exe"
    grep -q "$expected_signer" <<<"$report" || fail "signataire inattendu : $exe"
    grep -q -i "timestamp" <<<"$report" || fail "pas d'horodatage : $exe"
    printf '  ok  %s\n' "$(basename "$exe")"
}

step "Vérification des signatures"
for exe in "${exes[@]}"; do
    signed="$output/$(basename "$exe")"
    [ -f "$signed" ] || fail "$(basename "$exe") n'a pas été signé"
    verify "$signed"
    cp "$signed" "$exe"
done

# --- Zip, réextraction, empreinte ------------------------------------------
name=$(basename "$package")
case $name in
    *Windows*) default_zip="$name${version:+-$version}.zip" ;;
    *) default_zip="$name${version:+-$version}-Windows-x64.zip" ;;
esac
zip_name=${ZIP_NAME:-$default_zip}
mkdir -p "$out_dir"
out_dir=$(cd "$out_dir" && pwd)
zip_path="$out_dir/$zip_name"
rm -f "$zip_path"
(cd "$(dirname "$package")" && zip -qr -X "$zip_path" "$name")

step "Revérification dans le zip"
mkdir "$work/check"
unzip -q "$zip_path" -d "$work/check"
for exe in "${exes[@]}"; do verify "$work/check/$name/$(basename "$exe")"; done

step "Terminé"
echo "  $zip_path"
echo "  SHA-256 $(shasum -a 256 "$zip_path" | cut -d' ' -f1)"
