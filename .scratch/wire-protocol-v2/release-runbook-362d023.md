# Release runbook — Audio Share 0.5.0-beta

Target: annotated tag `v0.5.0-beta` → `362d023`
Bundle: `~/.cache/audio-share-beta-release/release-362d023/`
Status: **PUBLISHED as a pre-release** on 2026-11-18.

- URL: `https://github.com/W0n9/audio-share/releases/tag/v0.5.0-beta`
- `isDraft=false`, `isPrerelease=true`, publishedAt `2026-11-18T15:03:22Z`, 7 assets.

- Remote: `fork` = `git@github.com:W0n9/audio-share.git` (do **not** use `origin`; it is
  upstream `mkckr0/audio-share`).
- Remote tag: `refs/tags/v0.5.0-beta` → tag object `6d9363f2…` → commit `362d023`.
- Draft release: `https://github.com/W0n9/audio-share/releases/tag/untagged-49c91ed9993e54671e17`
  (draft id `450126052`, tagName `v0.5.0-beta`, 7 assets).
- All seven asset digests reported by the GitHub API match `SHA256SUMS` exactly.

## What is in the bundle

| File | SHA-256 |
|---|---|
| `audio-share-client-v2-0.2.0-beta.apk` | `2024ae1423090670f6b46b92277fbed39df40dcc8f0ad74d4b801fa44722677a` |
| `AudioShareServer_0.5.0-beta_amd64.deb` | `2b8a4ba246791b5f09a0b123ae7ba2b698c54561eca1d963ea039866678b5f9d` |
| `AudioShareServer_0.5.0-beta_amd64.AppImage` | `78e91548ab188784df97b40e253b905be12a15e89f5f5e0d7d1a628f80a7b085` |
| `audio-share-server-cmd-linux.tar.gz` | `66dd752065fd5ec54cfb429d6132d8866fcffd21cac369d17543cf63171aec5a` |
| `AudioShareServer.exe` | `78874511654260e77a6a5c40bcb0c504a71e8dcd81fb458bce22ab0816c6805c` |
| `audio-share-server-cmd-windows.zip` | `d5f9e977702670520547cc27b2633bbb9c443956fb25ababdd234cae4602e9a8` |

Plus `SHA256SUMS`, `release-notes.md`, `MANIFEST.txt`. `sha256sum -c SHA256SUMS` passes.

## Pick one publishing path

**A — upload these local artifacts (recommended for this release).**
The hashes above are what the release notes and manifest cite, and what JVM evidence was
produced from. Uploading these keeps everything consistent.

**B — let the CI workflow rebuild.**
`.github/workflows/release.yml` builds from source on `ubuntu-24.04` and creates a draft
release. Rebuilds are **not byte-reproducible**: the resulting SHA-256 values will differ
from the ones above, so you must regenerate `SHA256SUMS` and update every hash in the
release notes. Only choose B if you explicitly want CI-built artifacts; the WAIVED
evidence rationale in the release notes still applies, but the hash citations must change.
B also needs repository secrets: `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`,
`ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`.

## Preconditions

1. `gh auth status` shows a logged-in account with `repo` access.
2. `git status --short` is clean (the only untracked files may be `.scratch/`).
3. `git rev-parse v0.5.0-beta^{commit}` prints `362d023…` (annotated tag, local).
4. The bundle passes `sha256sum -c SHA256SUMS`.
5. Confirm the release notes still contain the `WAIVED` statement — do not publish a
   document that implies the final artifacts were re-verified on device.

## Publish (path A)

Steps 1–4 are **done** (2026-11-18). The release is published as a pre-release.

```bash
# 1. DONE — Push only the tag (the branch tip is a docs-only commit and need not ship).
git push fork v0.5.0-beta

# 2. DONE — Create a DRAFT release with the local artifacts.
gh release create v0.5.0-beta --repo W0n9/audio-share --draft \
  --title "Audio Share 0.5.0-beta" \
  --notes-file ~/.cache/audio-share-beta-release/release-362d023/release-notes.md \
  ~/.cache/audio-share-beta-release/release-362d023/audio-share-client-v2-0.2.0-beta.apk \
  ~/.cache/audio-share-beta-release/release-362d023/AudioShareServer_0.5.0-beta_amd64.deb \
  ~/.cache/audio-share-beta-release/release-362d023/AudioShareServer_0.5.0-beta_amd64.AppImage \
  ~/.cache/audio-share-beta-release/release-362d023/audio-share-server-cmd-linux.tar.gz \
  ~/.cache/audio-share-beta-release/release-362d023/AudioShareServer.exe \
  ~/.cache/audio-share-beta-release/release-362d023/audio-share-server-cmd-windows.zip \
  ~/.cache/audio-share-beta-release/release-362d023/SHA256SUMS

# 3. Inspect the draft in the browser: seven assets, SHA256SUMS, notes render correctly.
gh release view v0.5.0-beta --repo W0n9/audio-share

# 4. Publish when satisfied (irreversible-ish):
gh release edit v0.5.0-beta --repo W0n9/audio-share --draft=false
```

## Post-publish checks

1. Download every asset into a fresh directory; `sha256sum -c SHA256SUMS` passes.
2. Install the APK (`adb install` or on a phone) and launch once; the app shows
   `0.2.0-beta` and the paired-server/QR entry points.
3. Download the Windows exe on a Windows host and start it; `server.log` should contain
   `Audio Share Server 0.5.0-beta starting`.
4. If `server.log` lacks `event=session.closed … code=701` after a Stop, record it as the
   still-open ticket-33 verification (do not silently upgrade the WAIVED evidence).

## Rollback

- Before publishing: delete the draft release (`gh release delete v0.5.0-beta --draft`)
  and, if needed, the remote tag (`git push origin :refs/tags/v0.5.0-beta`).
- After publishing: the tag/release can be deleted the same way; v0.3.4 and v1 are
  unaffected. Do not rewrite the release notes in place without also regenerating hashes.

## Explicitly out of scope for this runbook

- F-Droid publication and in-app auto-update (not built).
- Pushing `main`.
- Re-running device/Windows/EMULATOR acceptance (WAIVED by the maintainer, ticket 24).
