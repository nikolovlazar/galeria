#!/usr/bin/env python3
"""Generate the GitHub Releases update feed from the final release assets."""

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import sys
from urllib.parse import quote


ASSETS = {
    "linux-x86_64": "Galeria-linux-x64.tar.gz",
    "macos-aarch64": "Galeria-macos-arm64.zip",
    "windows-x86_64": "Galeria-windows-x64-setup.exe",
}


def signing_key():
    encoded = os.environ.get("UPDATE_SIGNING_KEY")
    if not encoded:
        print("warning: UPDATE_SIGNING_KEY is unset; update assets will not be signed", file=sys.stderr)
        return None

    # cryptography is only needed when the release actually signs assets.
    from cryptography.hazmat.primitives import serialization
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

    key = Ed25519PrivateKey.from_private_bytes(base64.b64decode(encoded, validate=True))
    public = key.public_key().public_bytes(
        serialization.Encoding.Raw, serialization.PublicFormat.Raw
    )
    public_b64 = base64.b64encode(public).decode("ascii")
    print(f"GALERIA_UPDATE_PUBKEY: {public_b64}")
    expected = os.environ.get("GALERIA_UPDATE_PUBKEY")
    if expected and expected != public_b64:
        raise ValueError("GALERIA_UPDATE_PUBKEY does not match UPDATE_SIGNING_KEY")
    return key


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dist", type=Path, required=True, help="directory containing the final assets")
    parser.add_argument("--version", required=True, help="application version")
    parser.add_argument("--tag", required=True, help="GitHub release tag")
    parser.add_argument("--repository", required=True, help="GitHub owner/repository")
    args = parser.parse_args()

    key = signing_key()
    release = f"https://github.com/{args.repository}/releases"
    tag = quote(args.tag, safe="")
    platforms = {}
    for platform, name in sorted(ASSETS.items()):
        path = args.dist / name
        digest = hashlib.sha256()
        signature = None
        if key:
            # Ed25519 signs the complete asset, not its SHA-256 digest.
            data = path.read_bytes()
            digest.update(data)
            signature = base64.b64encode(key.sign(data)).decode("ascii")
            size = len(data)
            del data
        else:
            size = 0
            with path.open("rb") as asset:
                while chunk := asset.read(1024 * 1024):
                    digest.update(chunk)
                    size += len(chunk)
        entry = {
            "url": f"{release}/download/{tag}/{quote(name)}",
            "sha256": digest.hexdigest(),
            "size": size,
        }
        if signature is not None:
            entry["signature"] = signature
        platforms[platform] = entry

    feed = {"version": args.version, "page": f"{release}/tag/{tag}", "platforms": platforms}
    (args.dist / "latest.json").write_text(
        json.dumps(feed, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


if __name__ == "__main__":
    main()
