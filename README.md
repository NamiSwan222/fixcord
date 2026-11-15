# Hearth

A small chat app for friends on the same home network, written in Rust with egui.

- **Home Network community**: text channels shared with everyone running Hearth on the same Wi-Fi.
- **Voice chat** in the Lounge voice channel, with RNNoise noise suppression, voice activity or
  push to talk, echo reduction and per-friend volume.
- **Video and screen sharing**: camera, a whole screen or one app window, with quality and frame-rate
  choices.
- Friends and direct messages, replies, reactions, pins, search, themes, interface sounds and
  animations.
- **Automatic updates** from this repository's releases.

## Download

Get the newest `Hearth-x.y.z.zip` from [Releases](https://github.com/NamiSwan222/fixcord/releases),
unzip it and run `Hearth.exe`. Windows may warn that the program isn't signed: choose
"More info" → "Run anyway". Allow Hearth on private networks when the firewall asks.

## Build it yourself

```
cargo build --release
```

The program is `target/release/hearth.exe`.

## Publishing a new version

1. Raise `version` in `Cargo.toml` and build with `cargo build --release`.
2. Create a GitHub release whose tag is that version (for example `v0.2.3`) and attach `Hearth.exe`,
   `Hearth.exe.sha256` and the zip.
3. Copies of Hearth check for new releases by themselves and update.

`publish-release.ps1` does steps 2 and 3 for the files in `release/`.

## Open-source parts

Noise suppression uses nnnoiseless, a Rust version of RNNoise (BSD 3-Clause). Other libraries are
MIT and/or Apache-2.0; see `dist/Hearth/THIRD-PARTY-NOTICES.txt`.
