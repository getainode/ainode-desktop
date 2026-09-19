# Releasing AINode for macOS

The release pipeline lives in `.github/workflows/release.yml`. It builds a
universal (Apple silicon + Intel) `.app` on a GitHub-hosted Mac, wraps it in a
`.dmg` and a zip, and attaches both to the GitHub Release for the tag. Signing
and notarization are optional: when the Apple secrets exist the build is
signed and notarized, when they do not it still ships, unsigned.

## Cut a release

1. Bump the version in `src-tauri/tauri.conf.json` (keep `package.json` and
   `src-tauri/Cargo.toml` in step if they carry one). The tag must match this
   number or the workflow stops before building.
2. Land that on `main` through a PR, then tag and push:

   ```bash
   git checkout main && git pull
   git tag v0.1.0
   git push origin v0.1.0
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
  `https://github.com/getainode/ainode-desktop/releases/tag/v0.1.0`:
  `AINode_0.1.0_universal.dmg`, `AINode_0.1.0_universal.app.zip`, and a
  `.sha256` file for both. The release is created by the workflow with
  generated notes; edit the notes afterwards if you want.

  ```bash
  gh release view v0.1.0 --repo getainode/ainode-desktop
  gh release download v0.1.0 --repo getainode/ainode-desktop --pattern '*.dmg'
  ```

- **The workflow run's artifacts** (Actions tab, bottom of the run page), as
  `AINode-0.1.0-macos-universal`. These are kept for 30 days and are the only
  output of a `workflow_dispatch` run started without a tag.

## Re-run a release

`workflow_dispatch` takes an optional `tag` input. Start it with an existing
tag to rebuild that tag and replace the files on its release. This is how to
turn an unsigned release into a signed one after the secrets are added, with
no new tag:

```bash
gh workflow run release.yml --repo getainode/ainode-desktop -f tag=v0.1.0
```

Started without a tag it builds the current branch and only uploads workflow
artifacts, which is the way to smoke-test the pipeline.

## Signing and notarization (optional)

Tauri reads six environment variables. The workflow passes them through from
repository secrets of the same names, and only when they are set, so a repo
with no secrets builds unsigned rather than failing.

| Secret | What it is |
| --- | --- |
| `APPLE_CERTIFICATE` | The Developer ID Application certificate, a `.p12` export, base64 encoded |
| `APPLE_CERTIFICATE_PASSWORD` | The password chosen when the `.p12` was exported |
| `APPLE_SIGNING_IDENTITY` | The certificate's common name, `Developer ID Application: <name> (<team id>)` |
| `APPLE_ID` | The Apple ID (email) of the developer account |
| `APPLE_PASSWORD` | An app-specific password for that Apple ID, not the account password |
| `APPLE_TEAM_ID` | The 10 character team id from the Apple Developer membership page |

Signing needs the first three. Notarization needs all six; with only the first
three the app is signed but not notarized and the run prints a warning.

### Where the material lives

Everything is in Bitwarden. Do not put a value in a file that could be
committed, in a chat, or in a PR.

- The Developer ID certificate (`.p12`) is an **attachment on the
  "Apple Developer" item**. The export password and the team id are on the
  same item.
- The App Store Connect API key (`.p8`) is on the **`titanium-bot-ci`** item.
  The workflow does not use it: notarization is wired through the Apple ID and
  an app-specific password. Tauri also accepts the API key route
  (`APPLE_API_ISSUER`, `APPLE_API_KEY`, `APPLE_API_KEY_PATH`); switching to it
  means writing the `.p8` to disk in the workflow, so it is a small change if
  the app-specific password route ever becomes a nuisance.

### Add the secrets

Pull the `.p12` from Bitwarden to a scratch location, then:

```bash
export REPO=getainode/ainode-desktop

# The certificate, base64 in one line. macOS base64 does not wrap by default.
base64 -i DeveloperID.p12 | gh secret set APPLE_CERTIFICATE --repo "$REPO"

# The rest are prompted for on stdin; nothing lands in shell history.
gh secret set APPLE_CERTIFICATE_PASSWORD --repo "$REPO"
gh secret set APPLE_SIGNING_IDENTITY --repo "$REPO"
gh secret set APPLE_ID --repo "$REPO"
gh secret set APPLE_PASSWORD --repo "$REPO"
gh secret set APPLE_TEAM_ID --repo "$REPO"

gh secret list --repo "$REPO"
rm DeveloperID.p12       # remove the scratch copy
```

To read the signing identity off the certificate without importing it:

```bash
openssl pkcs12 -in DeveloperID.p12 -nokeys -clcerts 2>/dev/null \
  | openssl x509 -noout -subject
```

The `CN=` part of the subject is the identity. If `openssl` is OpenSSL 3
(Homebrew) and complains about an unsupported algorithm, add `-legacy` to the
`pkcs12` call; a Keychain Access export uses older ciphers that the stock
macOS LibreSSL still reads without it. An app-specific password is
created at account.apple.com under Sign-In and Security.

## Unsigned builds and Gatekeeper

Until the secrets are in place every release is unsigned. macOS quarantines
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
variables in your shell before running it, or leave them unset for an
unsigned build.

## App icon

`src-tauri/icons/` is generated from the AINode mark (the green hex lattice
from the product web UI) placed on a macOS-style tile. To regenerate it:

```bash
python3 scripts/make-icon.py path/to/ainode-logo.png icon.png
npx --yes @tauri-apps/cli@latest icon icon.png -o src-tauri/icons/
```

The 1024 px source is kept at `src-tauri/icons/icon-1024.png`.

Made in Texas.
