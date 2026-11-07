# Releasing sandb

sandb uses Semantic Versioning (`MAJOR.MINOR.PATCH`). `package.json` holds the one version number. `src-tauri/tauri.conf.json` reads it from there.

## One-time setup

1. The updater signing key is at `~/.tauri/sandb.key` (private) and `~/.tauri/sandb.key.pub` (public, already in `tauri.conf.json`).
   Keep a backup of the private key, for example in a password manager. If you lose it, installed apps can never update again.
2. In GitHub, open the repo, then Settings > Secrets and variables > Actions, and add:
   - `TAURI_SIGNING_PRIVATE_KEY`: the full text of `~/.tauri/sandb.key`.
   Do not create `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. The key has no password.
   - `MACOS_CODESIGN_CERTIFICATE`: the output of `base64 -i ~/.tauri/sandb-codesign.p12`.
   - `MACOS_CODESIGN_CERTIFICATE_PASSWORD`: the text of `~/.tauri/sandb-codesign.p12.password`.
3. The code signing certificate is at `~/.tauri/sandb-codesign.p12`. It is self-signed, with the name `sandb Code Signing`, and it is valid for 10 years.
   The workflow signs the app with it. macOS does not trust a self-signed certificate, so it keeps a Keychain "Always Allow" permission for one build only (the cdhash), not for the certificate.
   Thus macOS asks for Keychain access one time after each update. sandb keeps one Keychain entry (the key of `secrets.enc`), so this is one prompt. A Developer ID certificate from Apple is necessary to remove it.
   Keep a backup of the `.p12` file and its password.
4. The repo must be public. The app downloads `latest.json` from `https://github.com/ihsanbudiman/sandb/releases/latest/download/latest.json`.

## Release a new version

1. Change `version` in `package.json` (for example `0.1.0` to `0.2.0`).
   Optional: set the same version in `src-tauri/Cargo.toml`. The app does not read it.
2. In `CHANGELOG.md`, move the `[Unreleased]` items under a new `## [0.2.0] - <date>` heading.
3. Commit, tag and push:

   ```sh
   git commit -am "Release v0.2.0"
   git tag v0.2.0
   git push origin main v0.2.0
   ```

4. GitHub Actions (`.github/workflows/release.yml`) builds the app and publishes the release. The workflow stops if the tag does not match `package.json`, or if `CHANGELOG.md` has no `## [x.y.z]` section for the tag. The items of that section become the release notes on GitHub and in `latest.json`.
5. Installed apps see the update at the next start, or with Settings > Updates > Check for updates.
6. On the first start after the update, the app shows a "What's new" dialog with the `CHANGELOG.md` sections from the last version that was opened up to the new version. The app bundles `CHANGELOG.md` at build time, so a section must exist before the build.

## Local build

A release builds and publishes both macOS architectures: Apple Silicon (`aarch64-apple-darwin`) and Intel (`x86_64-apple-darwin`).

`createUpdaterArtifacts` is on, so a local `pnpm tauri build` needs the private key:

```sh
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/sandb.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
pnpm tauri build
```

To build one architecture:

```sh
pnpm build:mac-arm    # Apple Silicon
pnpm build:mac-intel  # Intel
```

Both scripts need the signing key above. Without the key, turn the updater artifacts off in the same command:

```sh
pnpm tauri build --target x86_64-apple-darwin --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

Cross-compiling the other architecture needs its Rust target once: `rustup target add x86_64-apple-darwin` (on an Intel Mac) or `rustup target add aarch64-apple-darwin` (on Apple Silicon). The bundles land in `src-tauri/target/<target>/release/bundle/`; the Intel disk image is named `sandb_<version>_x64.dmg`.

## Notes

- Releases include an Apple Silicon and an Intel build, neither is notarized. A new user must right-click the app and choose Open the first time.
- A local `pnpm tauri build` is ad-hoc signed. Like a release update, each new build asks one time for Keychain access.
- Before the update installs, sandb asks you to commit or roll back open transactions.
