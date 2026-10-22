#!/usr/bin/env python3
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys


sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("moss_runtime_patch", HERE / "apply-runtime-patch.py")
patching = importlib.util.module_from_spec(spec)
spec.loader.exec_module(patching)
EXPECTED_RUNTIME_INFO = {
    "runtime": "mlx-audio-swift-v0.1.3+nero-moss-whole-v1",
    "defaultPolicy": "whole-recording-v1",
    "maxInputSeconds": 5400,
    "maxRequestedTokens": 65536,
}
BUNDLES = ("mlx-swift_Cmlx", "swift-transformers_Hub", "swift-crypto_Crypto")
METAL_RESOURCE = "mlx-swift_Cmlx.bundle/Contents/Resources/default.metallib"


def regular_file(path):
    path = Path(path)
    if path.is_symlink() or not path.is_file():
        raise ValueError("A required package input is missing or symlinked.")
    return path


def file_sha256(path):
    digest = hashlib.sha256()
    with regular_file(path).open("rb") as source:
        for block in iter(lambda: source.read(64 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def read_json(path):
    path = regular_file(path)
    if path.stat().st_size > 65536:
        raise ValueError("Package metadata is too large.")
    value = json.loads(path.read_text())
    if not isinstance(value, dict):
        raise ValueError("Expected a package metadata JSON object.")
    return value


def worker_source_hashes(source_root):
    source_root = Path(source_root)
    folder = source_root / "Sources/MossHarness"
    if folder.is_symlink() or not folder.is_dir():
        raise ValueError("The canonical worker sources are missing or symlinked.")
    paths = [source_root / "Package.swift"]
    for item in sorted(folder.rglob("*")):
        if item.is_symlink():
            raise ValueError("Symlinked worker source entries are not supported.")
        if item.suffix == ".swift":
            paths.append(regular_file(item))
    return {
        path.relative_to(source_root).as_posix(): file_sha256(path)
        for path in sorted(paths)
    }


def bundle_hashes(root):
    bundles = {}
    for name in BUNDLES:
        folder = root / (name + ".bundle")
        if folder.is_symlink() or not folder.is_dir():
            raise ValueError("A required worker resource bundle is missing or symlinked.")
        records = []
        for item in sorted(folder.rglob("*")):
            if item.is_symlink():
                raise ValueError("Symlinked resource bundle contents are not supported.")
            if item.is_file():
                records.append({"path": item.relative_to(folder).as_posix(), "sha256": file_sha256(item), "bytes": item.stat().st_size})
            elif not item.is_dir():
                raise ValueError("Unexpected resource bundle entry.")
        if not records:
            raise ValueError("A required worker resource bundle is empty.")
        encoded = json.dumps(records, sort_keys=True, separators=(",", ":")).encode()
        bundles[name + ".bundle"] = {"sha256": hashlib.sha256(encoded).hexdigest(), "fileCount": len(records)}
    regular_file(root / METAL_RESOURCE)
    return bundles


def verify_inputs(products, checkouts, patch_receipt, *, source_root=HERE,
                  work_root=patching.WORK_ROOT, git_command=patching.git):
    products = Path(products)
    checkouts = Path(checkouts)
    checkout = patching.owned_path(checkouts / "mlx-audio-swift", work_root)
    patch_receipt = patching.owned_path(patch_receipt, work_root)
    if patch_receipt.is_relative_to(checkout):
        raise ValueError("The patch receipt must be outside the dependency checkout.")
    contract, _ = patching.load_contract()
    if Path(git_command(checkout, "rev-parse", "--show-toplevel").strip()).resolve() != checkout.resolve():
        raise ValueError("Expected the patched MLXAudio checkout root.")
    if git_command(checkout, "rev-parse", "HEAD").strip() != contract["upstreamCommit"]:
        raise ValueError("The MLXAudio dependency commit is not pinned.")
    expected_status = " M " + contract["sourceFile"] + "\n"
    if git_command(checkout, "status", "--porcelain=v1", "--untracked-files=all", "--ignore-submodules=none") != expected_status:
        raise ValueError("The MLXAudio dependency has unexpected local changes.")
    source = patching.owned_path(checkout / contract["sourceFile"], work_root)
    if file_sha256(source) != contract["patchedSha256"]:
        raise ValueError("The MLXAudio source does not match the required runtime patch.")
    expected_receipt = dict(contract)
    expected_receipt["upstreamTree"] = git_command(checkout, "rev-parse", "HEAD^{tree}").strip()
    if read_json(patch_receipt) != expected_receipt:
        raise ValueError("The patch receipt does not match the current runtime manifest and checkout.")
    lock_path = Path(source_root) / "Package.resolved"
    lock = read_json(lock_path)
    pins = [pin for pin in lock.get("pins", []) if pin.get("identity") == "mlx-audio-swift"]
    if len(pins) != 1 or pins[0].get("state", {}).get("revision") != contract["upstreamCommit"] or pins[0].get("state", {}).get("version") != contract["upstreamVersion"]:
        raise ValueError("The canonical package lock does not match the runtime patch.")
    executable = regular_file(products / "MossHarness")
    if not os.access(executable, os.X_OK):
        raise ValueError("The MOSS worker is not executable.")
    worker_sources = worker_source_hashes(source_root)
    return {
        "runtimePatchManifest": contract,
        "patchProvenance": expected_receipt,
        "patchReceiptSha256": file_sha256(patch_receipt),
        "workerSourceSha256": worker_sources["Sources/MossHarness/MossHarness.swift"],
        "workerSourceFilesSha256": worker_sources,
        "packageResolvedSha256": file_sha256(lock_path),
        "inputExecutableSha256": file_sha256(executable),
        "resourceBundles": bundle_hashes(products),
        "mlxMetalResource": {"path": METAL_RESOURCE, "sha256": file_sha256(products / METAL_RESOURCE)},
    }


def runtime_info(staged, *, runner=subprocess.run):
    executable = regular_file(staged / "MossHarness")
    if not os.access(executable, os.X_OK):
        raise ValueError("The staged MOSS worker is not executable.")
    try:
        result = runner(
            [str(executable.resolve()), "--runtime-info"], cwd=str(staged.resolve()),
            stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=15, check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ValueError("The staged worker runtime metadata check did not complete.") from error
    if result.returncode or len(result.stdout) > 65536:
        raise ValueError("The staged worker runtime metadata check failed.")
    try:
        information = json.loads(result.stdout)
    except (ValueError, TypeError) as error:
        raise ValueError("The staged worker did not return runtime metadata JSON.") from error
    if information != EXPECTED_RUNTIME_INFO or type(information.get("maxInputSeconds")) is not int or type(information.get("maxRequestedTokens")) is not int:
        raise ValueError("The staged worker is stale or has a different runtime/policy contract.")
    return information


def finalize(products, checkouts, staged, patch_receipt, *, source_root=HERE,
             work_root=patching.WORK_ROOT, git_command=patching.git, runner=subprocess.run):
    staged = Path(staged)
    if staged.is_symlink() or not staged.is_dir():
        raise ValueError("Expected a real staged worker directory.")
    destination = staged / "RuntimeProvenance.json"
    if destination.exists() or destination.is_symlink():
        raise ValueError("Refusing to replace an existing runtime provenance record.")
    provenance = verify_inputs(
        products, checkouts, patch_receipt, source_root=source_root,
        work_root=work_root, git_command=git_command,
    )
    if file_sha256(staged / "Package.resolved") != provenance["packageResolvedSha256"]:
        raise ValueError("The staged package lock differs from the canonical lock.")
    if bundle_hashes(staged) != provenance["resourceBundles"]:
        raise ValueError("The staged resources differ from the selected build products.")
    provenance.update({
        "schemaVersion": 1,
        "runtimeInfo": runtime_info(staged, runner=runner),
        "executableSha256": file_sha256(staged / "MossHarness"),
        "signatureStage": "workerAdHocBeforeDeveloperId",
        "sourceBinding": "recordedAtPackagingNotEmbeddedInExecutable",
    })
    with destination.open("x") as output:
        json.dump(provenance, output, indent=2)
        output.write("\n")
        output.flush()
        os.fsync(output.fileno())
    return provenance


def main():
    parser = argparse.ArgumentParser(description="Verify the pinned MOSS worker package without loading a model.")
    subparsers = parser.add_subparsers(dest="operation", required=True)
    preflight = subparsers.add_parser("preflight")
    preflight.add_argument("products", type=Path)
    preflight.add_argument("checkouts", type=Path)
    preflight.add_argument("patch_receipt", type=Path)
    packaged = subparsers.add_parser("finalize")
    packaged.add_argument("products", type=Path)
    packaged.add_argument("checkouts", type=Path)
    packaged.add_argument("staged", type=Path)
    packaged.add_argument("patch_receipt", type=Path)
    arguments = parser.parse_args()
    try:
        if arguments.operation == "preflight":
            verify_inputs(arguments.products, arguments.checkouts, arguments.patch_receipt)
        else:
            finalize(arguments.products, arguments.checkouts, arguments.staged, arguments.patch_receipt)
    except (ValueError, OSError, KeyError, TypeError) as error:
        parser.exit(2, str(error) + "\n")


if __name__ == "__main__":
    main()
