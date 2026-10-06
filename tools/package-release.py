"""Package an already-built native release candidate with notices and checksums."""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import shutil
import subprocess
import tomllib
import uuid
import zipfile


def sha(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--toolchain", default="1.98.1")
    parser.add_argument("--output-root", type=Path, default=Path("target/dist"))
    options = parser.parse_args()
    workspace = Path(__file__).resolve().parent.parent
    version = tomllib.loads((workspace / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    binary = options.binary.resolve(strict=True)
    metadata = json.loads(subprocess.check_output(["cargo", "+" + options.toolchain, "metadata", "--locked", "--offline", "--format-version", "1", "--filter-platform", options.target], cwd=workspace))
    root = options.output_root.resolve() / str(uuid.uuid4())
    root.mkdir(parents=True)
    name = f"spanforge-verify-v{version}-{options.target}"
    bundle = root / name
    bundle.mkdir()
    shutil.copy2(binary, bundle / binary.name)
    if binary.stem == "spanforge-verify":
        legacy = binary.with_name("cliverifyr" + binary.suffix)
        if not legacy.is_file():
            raise SystemExit("Build the cliverifyr compatibility binary before packaging")
        shutil.copy2(legacy, bundle / legacy.name)
    for filename in ["LICENSE", "NOTICE", "README.md", "Cargo.lock"]:
        shutil.copy2(workspace / filename, bundle / filename)
    shutil.copytree(workspace / "examples", bundle / "examples")
    shutil.copytree(workspace / "docs", bundle / "docs")
    dependencies = []
    missing_license_texts = []
    licenses = bundle / "third-party-licenses"
    licenses.mkdir()
    for package in metadata["packages"]:
        if package["source"] is None:
            continue
        source = Path(package["manifest_path"]).parent
        destination = licenses / f'{package["name"]}-{package["version"]}'
        destination.mkdir()
        files = sorted(set(source.glob("LICENSE*")) | set(source.glob("COPYING*")) | set(source.glob("NOTICE*")))
        files = [path for path in files if path.is_file()]
        for path in files:
            shutil.copy2(path, destination / path.name)
        if not files:
            missing_license_texts.append(destination.name)
        dependencies.append({"name": package["name"], "version": package["version"], "license": package["license"], "source": package["source"], "repository": package["repository"]})
    (bundle / "dependency-inventory.json").write_text(json.dumps({"format": "spanforge-verify-dependencies-v1", "target": options.target, "scope": "resolved dependencies including build/test dependencies", "packages": dependencies, "missing_license_texts": missing_license_texts}, indent=2) + "\n", encoding="utf-8")
    rust = subprocess.check_output(["rustc", "+" + options.toolchain, "--version", "--verbose"], text=True)
    (bundle / "provenance.json").write_text(json.dumps({"release_candidate": True, "signed": False, "version": version, "target": options.target, "rust": rust, "host": platform.platform(), "binary_sha256": sha(binary), "cargo_lock_sha256": sha(workspace / "Cargo.lock"), "license_review_required": bool(missing_license_texts)}, indent=2) + "\n", encoding="utf-8")
    sums = [f"{sha(path)}  {path.relative_to(bundle).as_posix()}" for path in sorted(bundle.rglob("*")) if path.is_file()]
    (bundle / "SHA256SUMS").write_text("\n".join(sums) + "\n", encoding="utf-8")
    archive = root / (name + ".zip")
    with zipfile.ZipFile(archive, "x", zipfile.ZIP_DEFLATED, strict_timestamps=False) as output:
        for path in sorted(bundle.rglob("*")):
            if path.is_file():
                output.write(path, path.relative_to(root))
    (root / "SHA256SUMS").write_text(f"{sha(archive)}  {archive.name}\n", encoding="utf-8")
    print("Release candidate:", archive)


if __name__ == "__main__":
    main()
