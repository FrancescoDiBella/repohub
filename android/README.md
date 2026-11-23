# RepoHub for Android

The Android app is RepoHub's own interface (the Rust code in `src/`, compiled to WebAssembly and
drawn with WebGPU) in a full-screen WebView, plus a small Java layer that does on the phone what
`repohub-server` does for the web build. It needs no server.

| Part | Where | What it does |
|---|---|---|
| The page | `app/src/main/assets/web` (built, not in the repository) | The interface. On a phone it uses the layout in `src/mobile.rs` and the full-screen board editor in `src/notes_page.rs`. |
| `MainActivity` | `app/src/main/java/…` | Shows the page, serves its files from the APK, opens links in the browser, passes the back button to the page, and opens the system's "save to" and file pickers for a board's **Export** and **Import…** (`saveFile`, `openFile` on the bridge). |
| `Backend` | same | Answers the page's requests: the configuration, the account, GitHub, the drawings. |
| `GitHubPaths` | same | The GitHub API paths the app may call: the same list as `server/src/allowlist.rs`. Change both together. |
| `AccountStore` | same | The user name and the token. The token is encrypted with a key kept in the Android Keystore. |
| `Notes` | same | The drawings, as `.excalidraw` files in the app's private storage, laid out as on the server: conditional saves (`if=`), deletes into `trash/<date>/` for 30 days, every drawing with its git blob id (`api/notes/all`) and board sync's index (`api/notes/index`). |

## Where things are kept

- **The token** never reaches the page. The page asks for `gh/…`; `Backend` adds the token and
  sends the request to `https://api.github.com`, and only for the paths in `GitHubPaths`. A
  board's path in the notes repository keeps `%` signs in its file name (`%25XX` in the address),
  so `GitHubPaths.boardContentsPath` checks it strictly before anything is decoded; a board may
  send up to 16 MB, any other request 64 KB.
- **The page talks to the app** through `RepoHubNative.request(…)` (see `gh_fetch` in
  `src/net.rs`). The bridge answers only while the app's own page is on screen; any other
  address opens in the browser instead.
- **Backups** are off for the app: the token's key can't leave the phone, and drawings may be
  private.

## Build

You need Rust with the `wasm32-unknown-unknown` target, `wasm-bindgen-cli` 0.2.129, JDK 17,
Gradle 8.9 or later and the Android SDK (platform 35).

```sh
scripts/android-web.sh                        # builds the page into app/src/main/assets/web
gradle -p android test assembleRelease        # unit tests, then the APK
adb install android/app/build/outputs/apk/release/app-release.apk
```

CI does the same in the Desktop workflow (`.github/workflows/release.yml`, job `android`) and
attaches `RepoHub-<version>-android.apk` to the Latest build and to numbered releases.

## Signing

An APK must be signed, and Android installs an update only over an app signed with the same key.

- Without a key of yours, the build signs with Gradle's debug key. On CI that key is new for
  every build, so to install a newer APK you first remove the app (its account and drawings go
  with it).
- With your own key, updates install over the old app. Make one once, and keep it out of the
  repository:

  ```sh
  keytool -genkeypair -v -keystore repohub.jks -alias repohub -keyalg RSA -keysize 4096 -validity 10000
  base64 -w0 repohub.jks        # the value of ANDROID_KEYSTORE_BASE64
  ```

  Then add these repository secrets (Settings → Secrets and variables → Actions):
  `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`,
  `ANDROID_KEY_PASSWORD`. A local build reads the same names from the environment, with
  `ANDROID_KEYSTORE_FILE` pointing at the `.jks` file.

## Requirements and limits

- Android 12 or later, with Android System WebView 121 or later (WebGPU).
- No terminal, Claude chat, local clones, Changes tab or plugins: they need a computer.
- Drawings are kept on the phone. With a notes repository chosen in Settings → Account →
  Boards, they sync with your other devices through it: the page does the syncing, through
  `Backend` (the same `gh/…` requests as the web build, with the board paths in `GitHubPaths`).
  A board can also be exported and imported as an `.excalidraw` file.
- Pinch to zoom isn't there yet: the board editor has zoom buttons and a tool to move the view.
