"""Install the verified optional CPU HDR denoiser into this SyFrack checkout."""
from pathlib import Path
import hashlib
import tempfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
VERSION = "2.5.0"
NAME = f"oidn-{VERSION}.x64.windows"
URL = f"https://github.com/RenderKit/oidn/releases/download/v{VERSION}/{NAME}.zip"
# SHA-256 published for this asset by the official GitHub release API.
SHA256 = "6ae0474ef7606d68647c1e2c2842832d6af01128ee84a523d30368a91013b707"


def install():
    destination = (ROOT / "runtime" / "oidn").resolve()
    if not destination.is_relative_to(ROOT):
        raise RuntimeError("Runtime destination outside the checkout")
    if destination.exists():
        raise SystemExit(f"Runtime already exists: {destination}")
    staging_parent = ROOT / "target"
    staging_parent.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="oidn-install-", dir=staging_parent) as temp:
        staging = Path(temp).resolve()
        if not staging.is_relative_to(ROOT):
            raise RuntimeError("Staging destination outside the checkout")
        archive = staging / "runtime.zip"
        digest = hashlib.sha256()
        with urllib.request.urlopen(URL, timeout=60) as source, archive.open("wb") as output:
            while block := source.read(1024 * 1024):
                output.write(block)
                digest.update(block)
        if digest.hexdigest() != SHA256:
            raise RuntimeError("Official runtime checksum does not match")
        with zipfile.ZipFile(archive) as package:
            for item in package.infolist():
                if not (staging / item.filename).resolve().is_relative_to(staging):
                    raise RuntimeError("Archive path outside the staging directory")
            package.extractall(staging)
        source = (staging / NAME).resolve()
        if not source.is_relative_to(staging) or not (source / "bin" / "oidnDenoise.exe").is_file():
            raise RuntimeError("Incomplete runtime package")
        destination.parent.mkdir(exist_ok=True)
        source.rename(destination)
    print(f"Installed OIDN {VERSION}; exports use the CPU backend: {destination}")


if __name__ == "__main__":
    install()
