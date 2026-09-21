# AINode Desktop

AINode on your desktop: point it at your master node and go.

AINode Desktop is a small native app for macOS and Windows that wraps the [AINode](https://github.com/getainode/ainode) web UI. You tell it where your master AINode lives, and from then on the dashboard opens as its own app instead of a browser tab. It also puts your fleet in the menu bar and tells you when a model finishes loading.

It is a thin shell, not a rewrite. The window shows the same web UI your master already serves. The app adds the parts a browser cannot: one saved address, a menu bar item, native notifications, and a page that waits for the master when it is down.

## The one setting

Where is your master AINode?

Two fields sit behind that question:

- **Primary address**: `host:port` of the master node, for example `192.168.0.10:3000`.
- **Alternate address** (optional): a second way to reach the same node, like its tailnet address `100.122.26.9:3000`.

- **API key** (optional): only needed when the node has `ainode auth` turned on. Run `ainode auth key create` on the node and paste the key here; the app sends it as `Authorization: Bearer` on every request. A node that wants a key and has not been given one shows **needs a key** in the menu bar rather than **offline**, because it is up.

Each field has a **Test** button that calls `/api/status` and shows the node name and AINode version that answered. On launch the app probes both addresses at once (2 second timeout), uses whichever answers, and remembers which one worked so the next launch tries it first. The first launch with nothing saved opens this screen.

There is no third field, and there does not need to be. Every AINode routes every model the fleet serves, so any node can answer for the cluster. On every successful poll the app saves the fleet's own list of addresses (`endpoint_hint` on `/api/status`, or `GET /api/cluster/endpoint`, both of which every node answers). When neither configured address answers, it tries those saved nodes in order, master first, and keeps working from whichever one does: the Settings screen and the menu bar then say **Connected through Spark-2**. As soon as the primary answers again the app goes back to it. Nodes older than AINode 0.5.28 report no such list, and then the app behaves exactly as it did before: the two addresses and nothing else.

## https, when the node serves it

Nobody types `https`. A node with `ainode tls enable` says so in its own fleet list (`tls` and `tls_port` on every `endpoint_hint` row, AINode 0.5.30 and later), and from the next poll the app prefers that node's https port for the API and for the main window, with a lock in the menu bar title. The stored address stays the plain `host:port` the user typed: AINode never moves its HTTP port, so that is the stable thing to remember, and the scheme rides beside it.

The app verifies certificates and has no way not to. A certificate this Mac does not trust (a self-signed one, most likely) is reported as exactly that, with the fix: trust it on this machine, or give the node a real one with `ainode tls enable --tailscale`, which gets a Let's Encrypt certificate for the node's MagicDNS name and needs no trust-store work at all. The app then falls back to that node's http port for the rest of the session so the fleet stays usable while you fix it. There is no "accept anyway" button, and adding one would mean the API key above travelling to whoever answered.

Settings are stored in the app data folder (`~/Library/Application Support/ai.ainode.desktop/settings.json` on macOS, `%APPDATA%\ai.ainode.desktop\settings.json` on Windows). The saved fleet list lives in the same file, under `known`; it is a cache, not a setting, and pointing the app at a different address clears it.

## What you get

- **Main window**: the AINode web UI at `http://<master>/`. If that node is unreachable the window follows whichever node still answers, and only when nothing in the fleet does at all does a bundled "Waiting for ..." page take over and retry every 5 seconds. If the master restarts mid-session, the app notices within about 15 seconds, shows the waiting page, and goes back to the UI when the master answers again. Closing the window hides it; the app keeps running in the menu bar. Cmd+Q quits.
- **Menu bar item**: polls `/api/nodes` every 10 seconds and shows a title like `AINode · 6 nodes · 5 models`. The menu lists every node as `name · model or load progress · online/offline`, followed by Settings, Refresh, About and Quit.
- **Notifications**: when a node goes from loading to ready you get `Model is ready on Node (loaded in N min)`. When a node goes offline or comes back, you get one notification each way. Nothing fires on the first poll after launch.
- **About**: `AINode Desktop 0.2.0 · Made in Texas` and a link to [ainode.dev](https://ainode.dev).

The app only ever reads from the master (`GET /api/status`, `GET /api/nodes`, and `GET /api/cluster/endpoint` when a node's status carries no `endpoint_hint`), over https when that node advertises it, with the API key on every one of them. Everything you do inside the web UI goes through the UI itself, exactly as it would in a browser.

## Screenshots

The Settings screen, with the alternate address tested against a live master:

![AINode Settings](docs/settings.png)

The main window showing the AINode cluster view:

![AINode main window](docs/main.png)

The menu bar item with a six node fleet:

![Menu bar item](docs/tray.png)

## Build

Requirements: Rust 1.85 or newer, Node 20 or newer, and the Xcode Command Line Tools.

```bash
npm install
npm run tauri dev      # run against the source tree
npm run tauri build    # produce src-tauri/target/release/bundle/macos/AINode.app and a .dmg
```

Tests and lints live on the Rust side:

```bash
cd src-tauri
cargo test
cargo clippy
```

The build is unsigned. To open it on another Mac, right-click the app and choose Open the first time, or clear the quarantine flag with `xattr -dr com.apple.quarantine AINode.app`.

### Windows

The same source builds on Windows: Rust with the MSVC toolchain, Node 22 and the WebView2 runtime (part of Windows 10 and 11), then `npm ci` and `npm run tauri build` for an NSIS installer and an MSI under `src-tauri\target\release\bundle\`. On Windows the menu bar is File, View, Help; the fleet lives in the system tray (colour icon, double click opens the app); closing the window hides it to the tray. Releases carry signed installers built by `.github/workflows/release-windows.yml`; see [docs/RELEASING.md](docs/RELEASING.md#windows).

### Icons

`src-tauri/icons/` holds the standard Tauri icon set. To replace it, drop a square PNG (1024 px or larger) somewhere and run `npm run tauri icon path/to/icon.png`; no config changes are needed. `tray.png` and `tray@2x.png` are the monochrome macOS menu bar template icons; the Windows tray uses `icon.ico`.

### Layout

```
src/                    the app's own pages: settings, waiting, about (plain HTML, CSS, JS)
src-tauri/src/
  lib.rs                app wiring: plugins, menu, tray, windows, poller
  api.rs                the read-only calls, the API key, and why a probe failed
  config.rs             the settings, address normalization, the http-to-https
                        upgrade, store load and save
  probe.rs              which address to use, given what answered
  nodes.rs              /api/nodes parsing, tray labels, the notification state machine
  poller.rs             background loop: probe, navigate, poll nodes, notify
  tray.rs, menu.rs      menu bar item and the application menu
  windows.rs            main, settings and about windows
  commands.rs           IPC for the bundled pages
src-tauri/Info.plist    App Transport Security exemption so the webview may load plain http
```

### A note on plain http

AINode masters serve the UI over plain `http` on LAN and tailnet addresses, and that is still the default. macOS blocks plain http inside a webview unless the app declares `NSAllowsArbitraryLoadsInWebContent`. That single key lives in `src-tauri/Info.plist`. Do not add `NSAllowsLocalNetworking` or other keys next to it: macOS then ignores the broader exemption and the window goes blank. The key stays even on a fleet that has turned TLS on: HTTP on port 3000 is what every AINode always serves, it is the app's fallback, and it is what a node with no certificate yet answers on.

TLS uses `reqwest`'s `native-tls` feature rather than its default rustls, so verification goes through Security.framework on macOS and schannel on Windows. That is what makes "add the certificate to your trust store" work, and it needs no build dependency on either platform.

## Stack

- [Tauri 2](https://tauri.app) with the system WebKit webview
- Rust: `reqwest` (with `native-tls`, so the OS trust store decides), `serde`, `tokio`, `tauri-plugin-store`, `tauri-plugin-notification`, `tauri-plugin-opener`
- Plain HTML, CSS and JavaScript for the app's own screens. No framework, no bundler.

## License

Apache-2.0. See [LICENSE](LICENSE).

Made in Texas.
