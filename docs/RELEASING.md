# Releasing sandb

sandb uses Semantic Versioning (`MAJOR.MINOR.PATCH`). `package.json` holds the one version number. `src-tauri/tauri.conf.json` reads it from there.

## One-time setup

1. The updater signing key is at `~/.tauri/sandb.key` (private) and `~/.tauri/sandb.key.pub` (public, already in `tauri.conf.json`).
   Keep a backup of the private key, for example in a password manager. If you lose it, installed apps can never update again.
2. In GitHub, open the repo, then Settings > Secrets and variables > Actions, and add:
   - `TAURI_SIGNING_PRIVATE_KEY`: the full text of `~/.tauri/sandb.key`.
   Do not create `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. The key has no password.
3. The repo must be public. The app downloads `latest.json` from `https://github.com/ihsanbudiman/sandb/releases/latest/download/latest.json`.

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

`createUpdaterArtifacts` is on, so a local `pnpm tauri build` needs the private key:

```sh
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/sandb.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
pnpm tauri build
```

## Notes

- The build is Apple Silicon only and not notarized. A new user must right-click the app and choose Open the first time.
- Before the update installs, sandb asks you to commit or roll back open transactions.
