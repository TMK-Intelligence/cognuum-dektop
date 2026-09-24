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

## Native security and behavior

- Only the exact channel origin can navigate inside the main webview. Other HTTPS
  pages open in the system browser. File/javascript/other custom schemes are denied.
- Hosted pages receive **no Tauri IPC capabilities**. Native menus handle updates
  and settings. No remote shell, filesystem or arbitrary native commands exist.
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
