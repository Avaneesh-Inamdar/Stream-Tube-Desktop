# Stream Tube for Android

Stream Tube for Android brings the Stream Tube Desktop browsing interface to phones. The interface is built in Slint and the Android runtime uses RustFerry. The desktop project was created by Avaneesh Inamdar.

Home opens to a video feed with music, games, tech, and live categories. The bottom bar switches between Home, Shorts, Following, and Library. Search opens from the header. Tap a video to play it, or tap its channel name to see recent uploads and follow the channel.

Settings offers Russian, English, or the device language, plus a thumbnail toggle. Following, saved videos, and viewing history work locally. Connect with Google in Settings to read the YouTube account's subscriptions in Following.

## Build

Install Rust 1.92 or newer, JDK 17, Android SDK Platform 36, Build Tools 36, Android NDK 29, and Gradle 8.14.3. Set `ANDROID_HOME` and `ANDROID_NDK_HOME` to the SDK and NDK directories.

```powershell
rustup target add aarch64-linux-android x86_64-linux-android
cargo install cargo-ferry --version 0.1.0 --locked
cargo ferry check
cargo test --locked --lib
./tools/build-account-apk.ps1
```

Run the commands from `mobile/android`. The script builds the Rust libraries and Android bridge with RustFerry, packages Google Play services with the Android Gradle plugin, and signs `target/google-play/Stream-Tube-Android-0.1.0.apk` with the local RustFerry signing identity.

## Google authorization

Enable YouTube Data API v3 in Google Cloud. Create an Android OAuth client for `io.github.shiroksh.streamtube` and enter the SHA-1 fingerprint printed by the build. During testing, add the Google account to the project's OAuth test audience.

Sign-in uses the Google Play services authorization dialog. Stream Tube requests `youtube.readonly` and fetches the account's channel name and subscriptions. See the [privacy notice](PRIVACY.md) for details.

## Attribution

The Android source is MIT licensed. Slint is used under the [Slint Royalty-free License](https://slint.dev/agreements/slint-royalty-free-license.pdf) and credited in Settings. The release includes dependency licenses.

[![Made with Slint](https://slint.dev/logo/MadeWithSlint-logo-light.svg)](https://slint.dev/)
