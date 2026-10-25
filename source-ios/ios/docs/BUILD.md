# Building Glass for iOS

## Requirements

- macOS with Xcode 26 (iOS 26 SDK) and its command-line tools.
- Rust (stable, edition 2024 toolchain as for the desktop) with the iOS targets:
  `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`
- XcodeGen (`brew install xcodegen`) — the Xcode project is generated from `ios/project.yml`.
- Disk: a debug simulator build of the Rust core takes about 3 GB under `ios/target/`,
  a release device build about as much again.

Nothing else is downloaded at build time beyond the crates in `ios/core/Cargo.lock`
(and LAME, which is built from `vendor/lame-3.100`).

## Simulator

```sh
cd ios
xcodegen generate                       # first, and after adding or removing Swift files
xcodebuild -project Glass.xcodeproj -scheme Glass -configuration Debug \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro' build
```

The first step of the Glass target, "Build Rust core", runs `scripts/build-core.sh` for
the platform and configuration being built: it compiles the Rust core, regenerates the
Swift bindings and the C header, and puts `libglass_core.a` in Xcode's build products
folder, where the target links it from. So ⌘B / ⌘R in Xcode, or `xcodebuild`, is all
that's needed, and switching between simulator and device needs nothing by hand.
(`build-core.sh sim|device|all debug|release` can also be run on its own to build the
core without Xcode.)

Run with sample data, no account and no network: launch arguments `-demo` (in the
scheme's Run options or `xcrun simctl launch <device> app.glass.ios -demo`). Other
launch arguments are listed in STATUS.md.

## Device

```sh
cd ios
xcodebuild -project Glass.xcodeproj -scheme Glass -configuration Release \
  -destination 'generic/platform=iOS' build
```

Installing on an iPhone needs signing: open `Glass.xcodeproj`, select the Glass target,
Signing & Capabilities, choose your team (a free Apple ID works for personal builds,
with 7-day profiles). The bundle ID is `app.glass.ios`; change it if your team can't
use it. No signing identity or team is stored in the repository.

## Tests

```sh
# Rust: the core's own tests (host)
cd ios/core && cargo test --lib

# UI tests (simulator). The assistant tests need the mock OpenAI server:
cd ios
python3 -I scripts/mock-openai.py 8765 build/mock-openai.log &
TEST_RUNNER_GLASS_SHOTS=$PWD/build/shots xcodebuild -project Glass.xcodeproj -scheme Glass \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro' test
```

Screenshots from the UI tests land in `build/shots/` (and in the result bundle).

## Third-party notices

`python3 -I ios/scripts/licenses.py` regenerates `Glass/Resources/THIRD-PARTY.txt`
from the core's dependency graph (run it after changing dependencies).

## The desktop app

The desktop builds exactly as before (`cargo build` at the repository root). The shared
files differ on iOS only behind `cfg(glass_mobile)`, which the desktop's Cargo.toml
declares so it builds without warnings.
