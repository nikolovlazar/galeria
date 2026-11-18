# Updates and releases

Galeria uses GitHub Releases as its only update backend. It checks the static [`latest.json`](https://github.com/nikolovlazar/galeria/releases/latest/download/latest.json) asset on the latest release, compares its version with the running app, and offers a newer version in Settings. The feed links to the release page and stable-named platform assets, with each asset's size and SHA-256. An optional Ed25519 signature covers the raw asset bytes. Downloads are verified before installation. A checksum from the feed detects corruption, but does not by itself authenticate a feed or asset.

A `v*` tag push builds all platforms and publishes the assets, `SHA256SUMS.txt`, and `latest.json`. The feed version comes from `Cargo.toml`; make sure it matches the tag. The updater assets are:

| Feed platform | Asset | Installation |
| --- | --- | --- |
| `macos-aarch64` | `Galeria-macos-arm64.zip` | Replaces the app bundle and relaunches it |
| `windows-x86_64` | `Galeria-windows-x64-setup.exe` | Runs the Inno Setup installer silently |
| `linux-x86_64` | `Galeria-linux-x64.tar.gz` | Replaces the running executable and relaunches it |

In the app, an automatic check runs 15 seconds after launch and at most once every 24 hours; a failed attempt is also remembered for 24 hours in the cache so an offline machine does not retry on every launch. An available update appears in **Settings → General → Updates**, in the menu bar menu as **Install Update …**, and as a text badge beside the menu bar icon on macOS. **Install & Restart** downloads into the cache, checks size, SHA-256 and (when configured) the Ed25519 signature, verifies the staged app's code signature, swaps the bundle or executable, and relaunches. The check can be turned off, versions can be skipped, and **Check Now** always ignores the interval.

The macOS DMG, Windows portable ZIP and Debian package are also available for manual installation. A DMG is a good human installer, but the updater needs an archive containing the app bundle itself to stage and replace it without mounting a disk image. There is no macOS Intel updater asset in the release workflow. On Linux, self-updating only works when the executable is writable; package-managed installations may need a manual package upgrade instead. Windows and Linux updater coverage is x86-64 only.

## Signing and notarization

Without Apple credentials the app is signed ad hoc, is not notarized, and users may need to approve it in macOS security settings. With `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD` and `APPLE_TEAM_ID` configured as GitHub Actions secrets (and optionally `KEYCHAIN_PASSWORD`), CI imports the Developer ID certificate and signs the app with hardened runtime. It then submits a ZIP of the signed app to Apple's notary service, waits for approval, staples the ticket to the app, and checks it with `spctl`. Only then does `packaging/update/make_zip.sh` create the final updater ZIP. The DMG is built from the same already-stapled app and is signed and notarized separately. Locally, `packaging/update/make_zip.sh path/to/Galeria.app path/to/output.zip` wraps an existing app with `ditto`; `GALERIA_APP=path/to/Galeria.app packaging/macos/dmg.sh` uses that app without rebuilding it.

To authenticate update assets independently of platform signing, configure **both** repository settings under Settings → Secrets and variables → Actions:

- Variable `GALERIA_UPDATE_PUBKEY`: base64-encoded raw 32-byte Ed25519 public key. It is compiled into each platform build; an empty variable disables signature verification for that build.
- Secret `UPDATE_SIGNING_KEY`: base64-encoded raw 32-byte Ed25519 private seed. The release job uses it to sign every updater asset and prints the derived public key for comparison. It never prints the private key. A configured public key must match the seed or the release job fails.

Generate a pair on a trusted machine with Python's `cryptography` package installed:

```sh
python3 -c 'import base64; from cryptography.hazmat.primitives import serialization as s; from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey; k = Ed25519PrivateKey.generate(); print("UPDATE_SIGNING_KEY=" + base64.b64encode(k.private_bytes(s.Encoding.Raw, s.PrivateFormat.Raw, s.NoEncryption())).decode()); print("GALERIA_UPDATE_PUBKEY=" + base64.b64encode(k.public_key().public_bytes(s.Encoding.Raw, s.PublicFormat.Raw)).decode())'
```

Treat the seed and command output as secrets; store the seed in the Actions secret and do not commit it or paste it into logs. Set the public variable before building a release that should verify signatures. If the signing secret is absent, the feed has no signatures and CI warns; builds with an embedded public key will reject those updates. Builds without an embedded key do not authenticate signatures even when the feed includes them. Keep the signing key available for future releases: changing it requires a planned key transition for existing installations.

## Local feed testing

Prepare the three stable-named assets in `dist/`, then generate a feed with a version newer than the test app and rewrite its asset URLs to the local server. If the test app has a public key compiled in, export its matching `UPDATE_SIGNING_KEY` before generating the feed and install Python's `cryptography` package; otherwise leave the secret unset.

```sh
python3 packaging/update/latest_json.py --dist dist --version 99.0.0 --tag v99.0.0 --repository nikolovlazar/galeria
python3 -c 'import json, pathlib; p = pathlib.Path("dist/latest.json"); d = json.loads(p.read_text()); [a.update(url="http://127.0.0.1:8000/" + a["url"].rsplit("/", 1)[-1]) for a in d["platforms"].values()]; p.write_text(json.dumps(d, indent=2, sort_keys=True) + "\n")'
python3 -m http.server 8000 --directory dist
```

In another terminal, run the test app with `GALERIA_UPDATE_FEED=http://127.0.0.1:8000/latest.json`, or check from the CLI:

```sh
GALERIA_UPDATE_FEED=http://127.0.0.1:8000/latest.json /path/to/galeria update --check --json
```

HTTP is intended for local tests; do not use an untrusted feed. Use a disposable app copy when testing installation: it replaces the running bundle or executable. On macOS, move the app out of the DMG or Downloads into a writable location such as `/Applications` before updating. Apps running from a read-only location, without write permission on the parent directory, or under macOS App Translocation cannot replace themselves; install manually or move the app to a writable location and relaunch it.

## Rollback

To stop offering a broken update, remove its `latest.json` release asset, or publish/mark an older release as GitHub's latest and ensure it has its own feed and assets. Removing only an installer asset makes downloads fail rather than selecting another release. The updater only offers versions newer than the running version: publishing an older feed does not downgrade an already-updated app. Those users must install the older release manually, or wait for a newer fixed release.
