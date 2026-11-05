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
   The workflow signs the app with it. Because the signature does not change between versions, macOS keeps the Keychain "Always Allow" permission after an update.
   Keep a backup of the `.p12` file and its password. If you use a new certificate, each user must give Keychain access again one time.
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

4. GitHub Actions (`.github/workflows/release.yml`) builds the app and publishes the release. The workflow stops if the tag does not match `package.json`.
5. Installed apps see the update at the next start, or with Settings > Updates > Check for updates.

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
- A local `pnpm tauri build` is ad-hoc signed. Its signature is different from the release, so macOS asks again for Keychain access. To sign a local build like the release, import the `.p12` into a keychain and set `APPLE_SIGNING_IDENTITY="sandb Code Signing"`.
- Before the update installs, sandb asks you to commit or roll back open transactions.
