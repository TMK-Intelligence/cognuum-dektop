# Native window stability — 2026-10-16

Branch: `codex/multiwindow-stability`, from current `origin/main` (`3e5a0c7`,
remote refreshed during verification). Local changes only; no push or release.

`src-tauri/src/desktop.rs` now schedules workspace saving and geometry restoration
on the event loop. The window-state plugin holds its cache mutex while calling
native window getters. Previously workers could hold that mutex while waiting for
the event loop, whose focus/resize/close callbacks also needed the cache. New
windows start hidden and restore geometry on the event loop before showing.

`src-tauri/src/main.rs` skips the plugin's automatic restore for all eight workspace
labels, so its worker-side window-ready callback cannot reintroduce the lock cycle.
`scripts/workspace-threading.test.mjs` checks that contract and rejects planted
worker saves/restores using the same assertion helper.

Verification: Node checks 12 passed; Rust tests 22 passed, 1 explicitly ignored
(the optional external speech fixture); formatting and Clippy with `-D warnings`
passed; the Mac debug application bundle built. An isolated native diagnostic
with a distinct bundle identifier and local fallback pages completed five cycles
of eight windows: create, resize, focus, save, close secondary windows, reuse labels.
It exited 0 without a hang or crash. Diagnostic instrumentation was removed from
the delivered source. Voice models copied from the installed app passed the
repository's checksum verification.

The first Rust baseline run exposed a pre-existing timestamp collision in the
PDF test's temporary directory fixture; the unchanged test passed on rerun.
It was not a product PDF write failure. No PDF implementation was changed.

The companion platform branch `codex/native-controls-window-stability` fixes
WebKit Studio selects, native panel layout animation costs, flyover lifecycle,
and temporary chart canvas retention. Those changes require the normal web
promotion; this Rust change requires a new desktop release. Neither was published.

No authentication, origin allowlists, IPC capabilities, signed-update policy,
backend access, or migration changed. Windows compilation/runtime, authenticated
workspace recovery, MFA, sign-out, real multi-monitor restoration and a signed
installer/update were not exercised. No Cognuum crash report was available locally;
the lock-cycle repair does not establish the cause of every reported app exit.

Release follow-up: Windows CI exposed a CRLF-only failure in the planted worker
restore case. The mutation now tolerates either newline convention and asserts
that it actually changed the source. Separate LF and CRLF cases reproduce the
original test defect and pass with the corrected mutation. Runtime Rust code is
unchanged by this follow-up.
