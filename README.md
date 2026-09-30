# Cognuum desktop

Tauri 2 shell for **macOS and Windows desktops/laptops**. Repository spelling is
intentionally `cognuum-dektop`, as requested. The product is named Cognuum.

The production application loads **https://access.cognuum.com**. It reuses that
site's React UI, authentication, Supabase, Railway, entitlements and web releases.
There is no second product frontend, local database, Supabase key or privileged
credential in this repository. Native release changes are independent of ordinary
web UI deployments. The application requires an internet connection.

## Development

Requirements: Node 22, Rust (pinned by `rust-toolchain.toml`), Apple's command-line
tools on macOS; MSVC C++ build tools and WebView2 on Windows.

```sh
npm ci
npm run dev
```

`npm run dev` uses **production**. Do not use it to write test data.
`npm run dev:staging` uses https://staging-access.cognuum.com instead, with a
separate bundle identifier, cookie store, window state and deep-link scheme.
The dev-access host is intentionally never used (it uses the production backend).

```sh
npm run check
node scripts/voice-model.mjs
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run build -- --debug --bundles app
```

On Windows use `--bundles nsis`. For a universal Mac build first add both targets
(`rustup target add aarch64-apple-darwin x86_64-apple-darwin`), then build with
`--target universal-apple-darwin --bundles app,dmg`.
Unsigned/debug builds are internal development artifacts, not customer releases.
Supported product floor: macOS 13+, Windows 11 x64. Windows ARM and mobile are not
qualified. The native platform APIs vary, so CI compilation is not user-flow QA.

## Integration with the platform repository

The companion platform change adds Settings → Account → Desktop app at
`/console/settings?tab=account&section=desktop`, updates `get-download-url` to
this repository, and implements desktop-only PKCE OAuth. It must travel through
the platform's development → staging → production promotion process before those
new features appear in the production-loaded shell. Password login uses the
existing production flow already.

In the corresponding Supabase Auth redirect allowlists, configure exactly:

- Production: `cognuum://auth/callback`
- Staging: `cognuum-staging://auth/callback`

Do not use wildcard redirects. This is an operator configuration step; creating
this repository does not configure or deploy Supabase. OAuth opens the system
browser, returns only a single-use authorization code, and exchanges it with the
verifier retained in the originating webview's sessionStorage. The web application's
implicit/email-link auth flow and existing MFA checks remain unchanged. Closing
the app during sign-in deliberately requires starting again.

Persistent login reuses the platform's opt-in HttpOnly “Remember me” cookie.
Test a full process quit/relaunch on each OS; never restore persistent refresh
tokens through localStorage or unencrypted native files.

## Multiple windows

Sign in once, then use the native **Window** menu:

| Action | Mac | Windows |
| --- | --- | --- |
| New Window | Command+N | Ctrl+N |
| Open This Page in New Window | Command+Shift+N | Ctrl+Shift+N |
| Save Workspace | Command+Shift+S | Ctrl+Shift+S |

Up to eight independent windows can be arranged across monitors. Internal product
links that request a new window also open here. **Show All Windows** reveals hidden
windows. Closing the primary window keeps it available while other windows are
open; on macOS use **Quit Cognuum** to exit the process.

The app saves open product pages, window sizes and positions when focus changes
and on quit. After relaunch, the same account's workspace reopens after login and
MFA. Remember me retains its existing opt-in behavior. Sign-out clears the saved
workspace and closes secondary windows. Routes restore saved platform content;
unsaved forms, transient chart edits and in-page scroll positions are not captured.
Linked symbols, docking and named workspace presets are outside this first version.

This requires the companion platform workspace protocol (version 1). Older desktop
installers keep their original authentication behavior. Deploy the platform adapter
before publishing a desktop build that advertises this protocol.

## Native security and behavior

### Max voice (protocol 1)

The companion web adapter exposes **Max** only in supporting native builds, on
console/analysis pages, for signed-in members admitted by the existing seeded
`desktop_app` access gate. Voice starts **off**. Enable it in Max's settings:

Native windows use the **Max** control for chart dictation as well as navigation.
The unsupported browser speech API is not exposed in the desktop webview, so the
chart's browser-only microphone is hidden. Ordinary browsers are unaffected.

- Say **“Hey Max, open Analysis”** or **“Listen, load Apple”** in one breath.
  “Listen” can be disabled separately. Wake activation operates only while a
  Cognuum window is focused. A wake-only phrase leaves five seconds for a request.
- Hold **Ctrl+Shift+Space**, speak, then release. The chord is configurable on
  macOS and Windows. Turn off wake activation for shortcut-only microphone use.
- Escape cancels. Blur, page navigation, chart switches, account changes,
  sign-out and closing a window revoke the utterance. Only final transcripts run.

Speech runs **locally in English**, using the pinned Apache-2.0 sherpa-onnx 1.13.8
streaming Zipformer model (`en-2023-06-26`, int8). The model adds approximately
68 MiB of resources. `voice-model.mjs` checks the archive SHA-256 and each extracted
file; the vocabulary in `assets/voice-bpe.vocab` is exported from that archive's
`bpe.model` (500 pieces with their scores). A small phrase bias helps the recognizer
spell “Hey Max”; it does not permit non-anchored activations. Attribution and the
Apache license ship with the model. Build-time native libraries come from the
pinned sherpa-onnx crate's official release downloader.

After first enable, the model stays warm in memory until exit. Capture is
serialized across windows. Audio is bounded, discarded after recognition, never
logged, stored or sent to a cloud service. No transcription API key or per-minute
voice fee is introduced. Local navigation uses an exact destination allowlist;
chart requests reuse the existing omnibox fast path, AI entitlement/usage checks,
ambiguity handling and undo. Complex chart requests retain their existing AI cost.
Preferences persist, microphone consent does not persist across a page reload or
app restart. OS microphone permission is still required.

Validation before release: run synthetic speech fixtures and then test real
microphone wake detection, false activations, accents, Bluetooth device changes,
permission denial, CPU/battery use and end-to-end chart latency on both platforms.
CI compilation alone does not prove those behaviors. The ignored `speech_fixture`
Rust test accepts `COGNUUM_VOICE_FIXTURE` (local PCM WAV) and
`COGNUUM_VOICE_EXPECT` (expected transcript); it never records microphone audio.
Deploy the gated web adapter before publishing the 0.3 desktop release.

Generated intelligence PDFs use the platform's PDF renderer and a narrow native
save command (`downloads.protocol: 1`). Files go to the OS **Downloads** folder;
existing names receive a numbered suffix. The platform waits for the completed
write before reporting success. Failed writes report an error without retrying.
The command accepts only PDF bytes (64 MiB maximum) with a safe `.pdf` basename,
and cannot read files, accept a directory, fetch a URL or overwrite existing files.
Deploy the companion platform PDF adapter and install a build with this protocol
to enable it. Older installers continue using their browser download behavior.

- Only the exact channel origin can navigate inside the main webview. Other HTTPS
  pages open in the system browser. File/javascript/other custom schemes are denied.
- Only the exact channel origin in `main` / `workspace-*` receives eight narrow
  commands for in-memory session read/write, authentication locking and workspace
  readiness, PDF saving and opt-in voice sessions, plus scoped event subscription. Native menus own window
  creation and updates. No remote shell, general filesystem or window-management
  capability exists.
- All windows share one session in native process memory. Per-window sessionStorage
  mirrors remain; tokens never enter workspace files or localStorage. A native lock
  serializes Supabase refresh operations and releases abandoned leases on document
  navigation/window destruction. Notifications carry only a revision and event name.
- `workspace.json` stores the account ID and sanitized product routes; the window
  state plugin stores geometry. OAuth URLs, fragments and unapproved query fields
  are discarded. Workspace restoration never grants data access: existing server
  authorization and MFA remain authoritative.
- Updater signatures are separate from Apple/Windows code signing. Release builds
  receive a public updater key; private keys stay in protected CI secrets.
- Existing hosted CSP remains authoritative for remote pages, including PDFs and
  podcasts. The tiny bundled connection-error page has its own restrictive CSP.
- The startup connection timeout offers recovery through View → Reload Cognuum.
  Window state and standard edit shortcuts use native plugins/menus.
- Native detection is a cosmetic hint, never a replacement for server entitlement
  checks. Downloads still use the platform's verified JWT and subscription gate.

The fallback stylesheet and icon are snapshots of the existing platform tokens
and icon; edit product styling in the platform repository. No mobile project is
generated or maintained here.

## Signing and releases

This shell-only repository is **public** with the owner's approval. Installers
and signed updater manifests use public GitHub release assets; the application
and its data remain protected by the production platform's authentication and
entitlement checks. Never embed a GitHub token in the app.

GitHub environments `desktop-staging` and `desktop-production` are created and
restricted to the `main` branch. Signing credentials are not configured yet.
Configure these **secret names** (never commit values):

| Secret | Purpose |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri updater private key |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Updater key passphrase, if configured |
| `APPLE_CERTIFICATE` | Base64 Developer ID Application certificate export |
| `APPLE_CERTIFICATE_PASSWORD` | Certificate export password |
| `APPLE_ID`, `APPLE_PASSWORD` | Notarization account and app-specific password |
| `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_TENANT_ID` | Optional Windows signing-provider identity |

Environment **variables**: `TAURI_UPDATER_PUBLIC_KEY`, `APPLE_SIGNING_IDENTITY`,
`APPLE_TEAM_ID`, `WINDOWS_SIGN_SETUP`, `WINDOWS_SIGN_COMMAND`.

Generate and back up the updater key with `npm run tauri -- signer generate` using
an encrypted/password-manager-controlled location. The developer owns this key;
losing it requires shipping a new trusted app. Apple Developer organization
membership, certificate issuance and notarization access remain necessary even
with an existing D-U-N-S number.

For Windows choose a signing service that supports the company's legal
jurisdiction. `WINDOWS_SIGN_SETUP` installs its pinned CLI; `WINDOWS_SIGN_COMMAND`
is Tauri's custom signing command containing the `%1` file placeholder. Configure
only a reviewed command in these protected environments. For example, an eligible
Azure Artifact Signing account can use `artifact-signing-cli` with its endpoint,
account and certificate profile. Do not assume this service accepts every country.
The pipeline validates the resulting Authenticode signature.

After configuration, dispatch **Signed desktop release** from `main`. Both native
builds must pass, the Mac bundle must pass notarization-ticket validation, and
the Windows installer must carry a valid signature. Only then can the workflow
assemble a complete updater manifest and create a **draft** GitHub release.
Inspect installers on real machines before publishing. No workflow publishes
directly to customers.

Production uses immutable `v<version>` release tags; bump `package.json`,
`src-tauri/Cargo.toml`, the Cargo lockfile's application version, and
`src-tauri/tauri.conf.json` together. Staging uses the reserved `staging` release
tag for its isolated updater channel: an existing staging draft/published release
must be explicitly retired by the owner before creating its replacement. The
workflow refuses to overwrite an existing release. Never delete production tags.

The platform endpoint consumes the release's `.dmg`, `.exe` and `CHECKSUMS` block.
No first release means the Settings page displays a preparation message. Updates
must be tested from one signed version to the next, including rejection of a
tampered artifact; unit tests and unsigned app launches cannot establish that.

## Release acceptance

On a real Mac and Windows PC test: install/uninstall, password and Google login,
MFA, sign-out, remembered login after process restart, PDFs, chart export, uploads,
podcast audio, AI streaming, sleep/wake, network loss/reconnect, external links,
keyboard shortcuts, and update/restart. Confirm production release identity at
https://access.cognuum.com/api/release and record web SHA plus native version.
Browser automation alone is not verification of WKWebView/WebView2.

## Organization enrollment still required

The owner confirmed that the Apple and Windows signing accounts/certificates
are not available yet. Customer installers must wait for these credentials.

- [Enroll the organization in Apple Developer](https://developer.apple.com/programs/enroll/)
  using its existing D-U-N-S number. The authorized account holder completes the
  identity checks, agreement and membership purchase. Then obtain a Developer ID
  Application certificate and notarization credentials for distribution outside
  the Mac App Store.
- Obtain a Windows Authenticode certificate or managed signing service that
  accepts the organization's jurisdiction. Follow the [Tauri signing guide](https://v2.tauri.app/distribute/sign/windows/)
  and configure the CI signing commands above. A Microsoft Store listing is not
  required for direct installer downloads.

Account enrollment, contracts and purchases are owner actions. Do not publish
unsigned builds as production installers while waiting for approval.
# Unsigned preview installers

Until signing enrollment is complete, `Unsigned desktop preview` can build a
production-connected Mac universal DMG and Windows x64 installer from `main`.
Use an explicit version such as `0.1.0-preview.1`. The workflow creates a
**draft prerelease** with checksums and a prominent unsigned notice. Review both
assets before publishing; never mark this preview as the latest stable release.
These builds have no verified publisher signature or automatic updates, and
operating systems may block them. The signed release workflow remains separate
and still requires all signing credentials.
