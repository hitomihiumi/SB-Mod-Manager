# Known limitations

- **The virtual file system has not been tested against the game itself.**
  CI tests it end to end on Windows (and under Wine) with a stand-in process.
  It hooks Win32 file functions, so code calling `ntdll` directly sees the
  real folder. An Epic install may need the Epic launcher running.
- **Frame generation on RTX 20/30 is third-party.** The check reads the card
  and the game folder, but cannot prove the game will load the proxy.
- **The Nexus collection query was written without the live API.** The
  `collection.json` inside the revision archive is preferred when available.
- **Release checks use the anonymous GitHub API.** When it is rate limited,
  the app says it could not check.

Found something else? Open an issue and attach the diagnostics zip (Settings →
diagnostics).
