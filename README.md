# Dropfarmer

A desktop app for queuing Twitch Drops campaigns and tracking reward progress. Built with Svelte 5, Tauri 2, and Rust. Supports one Twitch account at a time.

- Sign in through a browser or an activation code.
- Search campaigns by game, reward, or streamer.
- Reorder the queue and skip campaigns that cannot currently earn progress.
- Claim completed rewards automatically or from Inventory.
- Load saved campaigns while fresh data loads in the background.

The Rust backend sends watch telemetry without downloading video. Progress shown in the app comes from Twitch. This is an unofficial integration and can break when Twitch changes its endpoints.

## Run locally

The current desktop setup targets Windows.

Install:

- [Bun](https://bun.sh/) — the project uses version 1.3.3.
- Node.js 22.12 or newer for the frontend tools.
- Rust through [rustup](https://rustup.rs/).
- Visual Studio C++ Build Tools and Microsoft Edge WebView2. See [Tauri's Windows prerequisites](https://v2.tauri.app/start/prerequisites/#windows).
- Chrome or Edge if you want browser sign-in.

Then run:

```powershell
bun install --frozen-lockfile
bun run tauri dev
```

Tauri starts the frontend server on port `1420`. If that port is already in use, stop the other development server before starting Tauri.

For a browser-only UI preview:

```powershell
bun run dev
```

Open `http://127.0.0.1:1420` and choose **Preview** to use sample data. Sign-in and farming require the desktop app. Run either the preview or Tauri, not both on the same port.

## Use

1. Select **Connect Twitch** and choose a sign-in method.
2. Link your game account through [Twitch's campaigns page](https://www.twitch.tv/drops/campaigns), then refresh the app.
3. Add campaigns to **My queue**. Use the arrows to set their order.
4. Select **Start farming**.

The app checks queued campaigns for an eligible live channel and refreshes progress roughly once a minute. Unlinked accounts, offline channels, future campaigns, and unmet reward prerequisites are skipped so later entries can run. Completed campaigns leave the queue automatically.

Automatic claiming is enabled by default. Turn it off in **Settings** to claim rewards manually from **Inventory**.

Closing the app stops farming. The queue is restored on the next launch, but stays paused until you start it. There is no system tray service or automatic startup.

### Sign-in methods

**Browser:** opens a separate Chrome or Edge profile. Complete Twitch's login and leave the campaigns page open until the app finishes connecting. The app checks campaign access before saving the session, then closes its browser and removes the temporary profile. It does not use your everyday browser profile.

**Code:** displays a code to approve on Twitch's activation page. This login may only return campaigns already in progress. If a campaign is missing, try browser sign-in.

Switch methods in **Settings → Twitch connection → Change sign-in**. A failed or cancelled attempt keeps your existing saved login.

Browser sessions expire and currently need another sign-in; automatic renewal is not implemented. Search only filters the campaigns Twitch has returned.

## Build

```powershell
bun run tauri build
```

Windows outputs:

```text
src-tauri/target/release/dropfarmer.exe
src-tauri/target/release/bundle/nsis/Dropfarmer_<version>_x64-setup.exe
```

## Development

```powershell
bun run check
bun run test
cargo test --manifest-path src-tauri/Cargo.toml
```

Use `bun run test`, which runs Vitest. `bun test` is a different test runner. Keep both `bun.lock` and `src-tauri/Cargo.lock` committed.

Tests that contact Twitch or open a browser are ignored by default. Run a specific one with:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml live_browser_launch_and_cleanup -- --ignored
```

That test opens a blank browser and checks cleanup. The authenticated discovery and queue-refresh tests use the app's saved session; they do not send watch events or claim rewards.

| File                              | Purpose                                               |
| --------------------------------- | ----------------------------------------------------- |
| `src/App.svelte`                  | Campaigns, queue, inventory, settings, and sign-in UI |
| `src/lib/TitleBar.svelte`         | Custom window controls                                |
| `src-tauri/src/engine.rs`         | Farming worker, queue, and saved session handling     |
| `src-tauri/src/twitch.rs`         | Twitch requests and campaign discovery                |
| `src-tauri/src/browser_login.rs`  | Browser sign-in and session capture                   |
| `src-tauri/src/campaign_cache.rs` | Local campaign cache                                  |
| `src-tauri/src/model.rs`          | Campaign data and reward eligibility                  |

The app runs its own Rust backend. It does not launch Python or another miner.

Edit `public/dropfarmer.svg` to change the app icon, then run `bun run icons` to regenerate the native assets.

## Local data

Login credentials are stored in Windows Credential Manager. Queue preferences and campaign data are stored in:

```text
%APPDATA%/app.dropfarmer.desktop/
  preferences.json
  campaigns.json
```

The campaign cache is scoped to the account and sign-in method. After session validation, saved campaigns appear while the app refreshes them. Failed refreshes leave the previous list available. Caches older than seven days are ignored, and farming and claiming still check fresh Twitch data before acting.

The activity log is kept in memory. Credentials are not written to the campaign cache or preferences file.

## Credits

Parts of the Twitch protocol integration are adapted from [DevilXD/TwitchDropsMiner](https://github.com/DevilXD/TwitchDropsMiner) and [rangermix/TwitchDropsMiner](https://github.com/rangermix/TwitchDropsMiner), both under the MIT license.

The UI uses Geist, Geist Mono, and Lucide icons. Source revisions and third-party licenses are in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
