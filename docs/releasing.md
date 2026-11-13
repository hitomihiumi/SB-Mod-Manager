# Releasing and forking

## Releases

The version lives only in `package.json`. Bump it on `master`, then push a
matching tag (`v0.3.0` for `0.3.0`). The workflow refuses a mismatch, because
such a release would never be offered as an update.

Nightlies build daily when `master` has moved since the last one, or on demand
from the *Release* workflow. Each is versioned as the next patch prerelease
(`0.7.2-nightly.202610310300`), so the nightly channel offers it.

## Update signing

Updates must be signed. A fork needs its own keypair:

1. Run `pnpm tauri signer generate -w .tauri/updater.key`.
2. Put the public key in `plugins.updater.pubkey` in `src-tauri/tauri.conf.json`.
3. Store the private key and its password as the secrets
   `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

Losing the private key means existing installs can no longer update.

## Discord presence

A fork can use its own application id: set `APP_ID` in
`src-tauri/src/discord.rs` and upload an art asset named `icon` to that
application's Rich Presence assets.

## Rebranding a fork

The visible name is `productName` and the window `title` in
`src-tauri/tauri.conf.json`. Keep `identifier` (`dev.sbmm.manager`) unless you
mean to start a new app: the installer and updater treat a new identifier as a
different application. The artwork lives in `docs/logo.png`; regenerate the
icons with `python3 scripts/make-icons.py`.
