# Releasing AINode for macOS

The release pipeline lives in `.github/workflows/release.yml`. It builds a
universal (Apple silicon + Intel) `.app` on a GitHub-hosted Mac, wraps it in a
`.dmg` and a zip, and attaches both to the GitHub Release for the tag. When the
Apple secrets exist (they do, since 0.1.1) the app and the disk image are
signed with the Developer ID certificate, notarized through the App Store
Connect API key, and stapled; when they do not the build still ships,
unsigned.

## Cut a release

1. Bump the version in `src-tauri/tauri.conf.json` (keep `package.json` and
   `src-tauri/Cargo.toml` in step if they carry one). The tag must match this
   number or the workflow stops before building.
2. Land that on `main` through a PR, then tag and push:

   ```bash
   git checkout main && git pull
   git tag v0.1.1
   git push origin v0.1.1
   ```

3. Wait for the run. It takes about 10 to 20 minutes, more when the Rust
   cache is cold or notarization is on:

   ```bash
   gh run watch --repo getainode/ainode-desktop
   gh run list --repo getainode/ainode-desktop --workflow release.yml
   ```

A tag with a hyphen in it (`v0.2.0-beta.1`) is published as a pre-release.

## Where the artifacts land

Two places, same files:

- **The GitHub Release** for the tag, at
  `https://github.com/getainode/ainode-desktop/releases/tag/v0.1.1`:
  `AINode_0.1.1_universal.dmg`, `AINode_0.1.1_universal.app.zip`, and a
  `.sha256` file for both. The release is created by the workflow with
  generated notes; edit the notes afterwards if you want.

  ```bash
  gh release view v0.1.1 --repo getainode/ainode-desktop
  gh release download v0.1.1 --repo getainode/ainode-desktop --pattern '*.dmg'
  ```

- **The workflow run's artifacts** (Actions tab, bottom of the run page), as
  `AINode-0.1.1-macos-universal`. These are kept for 30 days and are the only
  output of a `workflow_dispatch` run started without a tag.

## Re-run a release

`workflow_dispatch` takes an optional `tag` input. Start it with an existing
tag to rebuild that tag and replace the files on its release. This is how to
rebuild a release after a workflow fix, with no new tag:

```bash
gh workflow run release.yml --repo getainode/ainode-desktop -f tag=v0.1.1
```

Prefer a new tag for anything users may already have downloaded: a rebuilt
file under the same name has a different hash, and the `.sha256` on the
release changes with it.

Started without a tag it builds the current branch and only uploads workflow
artifacts, which is the way to smoke-test the pipeline.

## Signing and notarization

The workflow signs with the Developer ID certificate and notarizes through an
App Store Connect API key. Both switch on by themselves when the repository
secrets below exist, and a repo with no secrets builds unsigned rather than
failing. Tauri reads the `APPLE_*` names straight from the environment; the
workflow only exports each one when it is set, because an empty
`APPLE_CERTIFICATE` makes the bundler try to import an empty certificate.

| Secret | What it is | Bitwarden source |
| --- | --- | --- |
| `APPLE_CERTIFICATE` | The Developer ID Application certificate, a `.p12` export, base64 encoded on one line | "Apple Developer", attachment `Developer_Certificates.p12` (the `P12_BASE64` attachment is the same bytes already encoded) |
| `APPLE_CERTIFICATE_PASSWORD` | The password chosen when the `.p12` was exported | "Apple Developer", field `IMPORT_PASSWORD` |
| `APPLE_SIGNING_IDENTITY` | The certificate's common name, `Developer ID Application: <name> (<team id>)` | "Apple Developer", field `IDENTITY` |
| `APPLE_TEAM_ID` | The 10 character team id | "Apple Developer", field `TEAM_ID` |
| `APPLE_API_KEY` | The App Store Connect API key id (10 characters) | `titanium-bot-ci`, field `KEY_ID` |
| `APPLE_API_ISSUER` | The App Store Connect issuer id, a UUID | `titanium-bot-ci`, in the notes under "Issuer ID" |
| `APPLE_API_KEY_B64` | The API key's `.p8` file, base64 encoded on one line | `titanium-bot-ci`, attachment `AuthKey_<KEY_ID>.p8` |

Signing needs the first three. Notarization needs signing plus the three
`APPLE_API_*` secrets; with only the signing three the app is signed but not
notarized and the run prints a warning. `APPLE_TEAM_ID` is passed through when
present; Tauri only requires it on the Apple ID route, which this workflow
does not use (there is no app-specific password in the vault, and the API key
does not expire when an Apple ID password changes). The certificate expires in
February 2027 ("Apple Developer", field `CERT_EXPIRES`); after that, export a
new one and replace `APPLE_CERTIFICATE`.

### What the workflow does with them

1. Imports the `.p12` into a throwaway keychain that lives as long as the job
   (`security create-keychain`, `security import`, partition list, search
   list), prints `security find-identity -v -p codesigning` and fails fast if
   the identity is not in it. A bad certificate or password fails in seconds,
   not after the compile.
2. Decodes `APPLE_API_KEY_B64` to `$RUNNER_TEMP/AuthKey_<KEY_ID>.p8` (mode
   600), proves the key against the notary service with
   `xcrun notarytool history`, and exports `APPLE_API_KEY`, `APPLE_API_ISSUER`
   and `APPLE_API_KEY_PATH` for Tauri.
3. `tauri build` signs every binary inside out, submits the zipped `.app`,
   waits for the verdict, staples the ticket, then builds the `.dmg` from the
   stapled app and signs the image.
4. Notarizes and staples the `.dmg` itself. Tauri signs the image but does not
   notarize it, and a disk image with no ticket of its own is held by
   Gatekeeper on a Mac that cannot reach Apple at that moment.
5. Verifies before anything is uploaded: `codesign --verify --deep --strict`,
   `spctl --assess` on the app and the image (both must report
   `source=Notarized Developer ID`), and `xcrun stapler validate` on both.
6. Deletes the key file and the keychain, whatever happened.

### Where the material lives

Everything is in Bitwarden. Do not put a value in a file that could be
committed, in a chat, in a PR, or in shell history.

- The Developer ID certificate (`.p12`), its export password, the identity
  string and the team id are on the **"Apple Developer"** item.
- The App Store Connect API key (`.p8`), its key id and the issuer id are on
  the **`titanium-bot-ci`** item.

### Add or rotate the secrets

Pull the material from Bitwarden into a mode 700 scratch directory, check it,
feed each value to `gh secret set` on stdin, then delete the directory.
Nothing is echoed and nothing lands in shell history.

```bash
export REPO=getainode/ainode-desktop
export BW_SESSION="$(cat ~/.bw-session)"       # after: bw unlock --raw > ~/.bw-session
umask 077 && T="$(mktemp -d)"

item_id() { bw get item "$1" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])'; }
field()   { bw get item "$1" | python3 -c 'import json,sys; it=json.load(sys.stdin); print(next(f.get("value") or "" for f in it["fields"] if f["name"]==sys.argv[1]), end="")' "$2"; }
APPLE="$(item_id "Apple Developer")"
ASC="$(item_id titanium-bot-ci)"

# Fields to files, never to the terminal.
field "$APPLE" IMPORT_PASSWORD > "$T/p12pass"
field "$APPLE" IDENTITY        > "$T/identity"
field "$APPLE" TEAM_ID         > "$T/teamid"
field "$ASC"   KEY_ID          > "$T/keyid"
bw get item "$ASC" | python3 -c 'import json,sys,re; print(re.search(r"^[0-9a-f-]{36}$", json.load(sys.stdin)["notes"], re.M|re.I).group(0), end="")' > "$T/issuer"

# Attachments straight to disk.
bw get attachment Developer_Certificates.p12 --itemid "$APPLE" --output "$T/cert.p12"
bw get attachment "AuthKey_$(cat "$T/keyid").p8" --itemid "$ASC" --output "$T/AuthKey.p8"
```

Check the pair before trusting CI with it. Both checks are read-only and touch
nothing but a keychain you create and delete; never import into the login
keychain.

```bash
KC="$T/verify.keychain-db"; KCPASS="$(uuidgen)"
security create-keychain -p "$KCPASS" "$KC"
security import "$T/cert.p12" -k "$KC" -P "$(cat "$T/p12pass")" -T /usr/bin/codesign
security find-identity -v -p codesigning "$KC"     # expect "1 valid identities found" and the IDENTITY string
security delete-keychain "$KC"

xcrun notarytool history --key "$T/AuthKey.p8" --key-id "$(cat "$T/keyid")" --issuer "$(cat "$T/issuer")"
# "Successfully received submission history." means the key, id and issuer agree.
```

Then set the secrets and clean up:

```bash
base64 < "$T/cert.p12"   | tr -d '\n' | gh secret set APPLE_CERTIFICATE --repo "$REPO"
base64 < "$T/AuthKey.p8" | tr -d '\n' | gh secret set APPLE_API_KEY_B64 --repo "$REPO"
gh secret set APPLE_CERTIFICATE_PASSWORD --repo "$REPO" < "$T/p12pass"
gh secret set APPLE_SIGNING_IDENTITY     --repo "$REPO" < "$T/identity"
gh secret set APPLE_TEAM_ID              --repo "$REPO" < "$T/teamid"
gh secret set APPLE_API_KEY              --repo "$REPO" < "$T/keyid"
gh secret set APPLE_API_ISSUER           --repo "$REPO" < "$T/issuer"

gh secret list --repo "$REPO"                  # names and dates only; values are write-only
rm -rf "$T"
```

To read the identity off a `.p12` without a keychain:

```bash
openssl pkcs12 -in "$T/cert.p12" -nokeys -clcerts -passin "file:$T/p12pass" 2>/dev/null \
  | openssl x509 -noout -subject
```

The `CN=` part of the subject is the identity. If `openssl` is OpenSSL 3
(Homebrew) and complains about an unsupported algorithm, add `-legacy` to the
`pkcs12` call; a Keychain Access export uses older ciphers that the stock
macOS LibreSSL still reads without it.

## Unsigned builds and Gatekeeper

`v0.1.0` shipped before the secrets were in place and is unsigned; every
release from `v0.1.1` on is signed and notarized. An unsigned build is what a
fork or a repo without the secrets produces. macOS quarantines
the download and blocks the first launch with "AINode cannot be opened because
the developer cannot be verified" or "Apple could not verify AINode is free of
malware". The app is fine; this is only the missing signature. One-time fix
per Mac:

- **macOS 14 and earlier:** right-click (or Control-click) `AINode.app` in
  Finder, choose **Open**, then **Open** again in the dialog. From then on it
  opens normally.
- **macOS 15 and later:** the right-click route no longer works. Double-click
  the app once so macOS records the block, then open **System Settings >
  Privacy & Security**, scroll to the "AINode was blocked" line and press
  **Open Anyway**.
- **From a terminal**, either version:

  ```bash
  xattr -dr com.apple.quarantine /Applications/AINode.app
  ```

Signed and notarized releases open with no prompt at all.

## Build and check locally

```bash
scripts/build-local.sh              # universal .app and .dmg, prints the paths
scripts/build-local.sh --native     # this Mac's architecture only, no rustup needed
scripts/verify-bundle.sh            # finds the built .app under src-tauri/target
scripts/verify-bundle.sh path/to/AINode.app --require-signed
```

`verify-bundle.sh` reports the signature state (unsigned, ad-hoc, or the
Developer ID chain), the Gatekeeper verdict from `spctl --assess`, whether a
notarization ticket is stapled, and checks that the bundle id is
`ai.ainode.desktop`. Unsigned is reported, not failed, unless you pass
`--require-signed`.

The local build signs and notarizes the same way CI does: export the same
variables in your shell before running it (`APPLE_SIGNING_IDENTITY` with the
certificate in your login keychain, or `APPLE_CERTIFICATE` and
`APPLE_CERTIFICATE_PASSWORD` for Tauri to import it, plus `APPLE_API_KEY`,
`APPLE_API_ISSUER` and `APPLE_API_KEY_PATH` to notarize), or leave them unset
for an unsigned build. The local script does not notarize the `.dmg`; CI
does.

## App icon

`src-tauri/icons/` is generated from the AINode mark (the green hex lattice
from the product web UI) placed on a macOS-style tile. To regenerate it:

```bash
python3 scripts/make-icon.py path/to/ainode-logo.png icon.png
npx --yes @tauri-apps/cli@latest icon icon.png -o src-tauri/icons/
```

The 1024 px source is kept at `src-tauri/icons/icon-1024.png`.

Made in Texas.
