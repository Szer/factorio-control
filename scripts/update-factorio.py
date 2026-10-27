#!/usr/bin/env python3
"""Pin the official headless archive version and SHA-256 for image builds."""
import argparse
import hashlib
import os
from pathlib import Path
import re
import tempfile
import urllib.parse
import urllib.request


def release(version):
    if re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version) is None:
        raise ValueError("Expected a numeric Factorio version such as 2.1.23")
    url = f"https://factorio.com/get-download/{version}/headless/linux64"
    request = urllib.request.Request(url, headers={"User-Agent": "Factorio-Control-release-updater/1.0 (+https://github.com/Szer/factorio-control)"})
    digest = hashlib.sha256()
    with urllib.request.urlopen(request, timeout=60) as response:
        if urllib.parse.urlsplit(response.url).scheme != "https":
            raise ValueError("The game download must use HTTPS")
        size = 0
        while chunk := response.read(1024 * 1024):
            size += len(chunk)
            if size > 1024 * 1024 * 1024:
                raise ValueError("Game archive exceeds 1 GiB")
            digest.update(chunk)
    if size == 0:
        raise ValueError("Game archive is empty")
    return digest.hexdigest()


def atomic_text(path, value):
    with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, delete=False) as file:
        temporary = Path(file.name)
        try:
            file.write(value + "\n")
            file.flush()
            os.fsync(file.fileno())
            os.replace(temporary, path)
        finally:
            temporary.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version")
    args = parser.parse_args()
    checksum = release(args.version)
    root = Path(__file__).resolve().parents[1]
    atomic_text(root / ".factorio-version", args.version)
    atomic_text(root / ".factorio-sha256", checksum)
    print(f"Pinned Factorio {args.version}; review both release files before committing.")


if __name__ == "__main__":
    main()
