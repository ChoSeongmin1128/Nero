#!/usr/bin/env python3
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys


sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("moss_runtime_patch", HERE / "apply-runtime-patch.py")
patching = importlib.util.module_from_spec(spec)
spec.loader.exec_module(patching)
EXPECTED_RUNTIME_INFO = {
    "runtime": "mlx-audio-swift-v0.1.3+nero-moss-whole-v2",
    "defaultPolicy": "whole-recording-v1",
    "maxInputSeconds": 5400,
    "maxRequestedTokens": 65536,
    "metadataSchemaVersion": 2,
    "eosPolicy": "model-generation-config+tokenizer-v1",
    "samplingPolicy": "greedy-argmax-v1",
}
BUNDLES = ("mlx-swift_Cmlx", "swift-transformers_Hub", "swift-crypto_Crypto")
METAL_RESOURCE = "mlx-swift_Cmlx.bundle/Contents/Resources/default.metallib"
MACHO = {b'\xcf\xfa\xed\xfe', b'\xce\xfa\xed\xfe', b'\xfe\xed\xfa\xcf', b'\xfe\xed\xfa\xce', b'\xca\xfe\xba\xbe', b'\xbe\xba\xfe\xca'}


def same_json(value, expected):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False) == json.dumps(expected, sort_keys=True, separators=(",", ":"), allow_nan=False)


def hex_digest(value, length):
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{" + str(length) + "}", value) is not None


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
    if not same_json(read_json(patch_receipt), expected_receipt):
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
    if not same_json(information, EXPECTED_RUNTIME_INFO):
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


def package_files(root):
    files = {}
    for path in sorted(Path(root).rglob("*")):
        if path.is_symlink():
            raise ValueError("Symlinked worker package entries are not supported.")
        if path.is_file():
            if path.stat().st_nlink != 1:
                raise ValueError("Hard-linked worker package entries are not supported.")
            with path.open("rb") as stream:
                signable = stream.read(4) in MACHO
            files[path.relative_to(root).as_posix()] = {
                "sha256": file_sha256(path), "bytes": path.stat().st_size, "machO": signable,
            }
        elif not path.is_dir():
            raise ValueError("Unexpected worker package entry.")
    return files


def verify_staged(staged, *, source_root=HERE, execute_runtime=False, runner=subprocess.run):
    staged = Path(staged)
    if staged.is_symlink() or not staged.is_dir():
        raise ValueError("Expected a real staged worker directory.")
    provenance_path = regular_file(staged / "RuntimeProvenance.json")
    provenance = read_json(provenance_path)
    contract, _ = patching.load_contract()
    if (type(provenance.get("schemaVersion")) is not int or provenance.get("schemaVersion") != 1
            or provenance.get("signatureStage") != "workerAdHocBeforeDeveloperId"
            or provenance.get("sourceBinding") != "recordedAtPackagingNotEmbeddedInExecutable"):
        raise ValueError("The worker provenance format or signature stage is not supported.")
    if not same_json(provenance.get("runtimePatchManifest"), contract):
        raise ValueError("The worker runtime patch manifest is stale.")
    receipt = provenance.get("patchProvenance")
    if not isinstance(receipt, dict) or set(receipt) != set(contract) | {"upstreamTree"}:
        raise ValueError("The worker patch receipt is incomplete.")
    if not same_json({key: receipt[key] for key in contract}, contract) or not hex_digest(receipt.get("upstreamTree"), 40):
        raise ValueError("The worker patch receipt does not match the canonical patch.")
    encoded_receipt = (json.dumps(receipt, indent=2) + "\n").encode()
    if provenance.get("patchReceiptSha256") != hashlib.sha256(encoded_receipt).hexdigest():
        raise ValueError("The worker patch receipt digest does not match its full contents.")
    expected_sources = worker_source_hashes(source_root)
    if (provenance.get("workerSourceFilesSha256") != expected_sources
            or provenance.get("workerSourceSha256") != expected_sources["Sources/MossHarness/MossHarness.swift"]):
        raise ValueError("The worker was packaged from different canonical sources.")
    canonical_lock = regular_file(Path(source_root) / "Package.resolved")
    lock_hash = file_sha256(canonical_lock)
    if provenance.get("packageResolvedSha256") != lock_hash or file_sha256(staged / "Package.resolved") != lock_hash:
        raise ValueError("The staged package lock does not match the canonical lock.")
    pins = read_json(canonical_lock).get("pins", [])
    matching = [pin for pin in pins if isinstance(pin, dict) and pin.get("identity") == "mlx-audio-swift"]
    if len(matching) != 1 or matching[0].get("state", {}).get("revision") != contract["upstreamCommit"] or matching[0].get("state", {}).get("version") != contract["upstreamVersion"]:
        raise ValueError("The canonical runtime pin does not match its patch.")
    if provenance.get("resourceBundles") != bundle_hashes(staged):
        raise ValueError("The staged worker resource bundles have changed.")
    metal = provenance.get("mlxMetalResource")
    if not isinstance(metal, dict) or metal != {"path": METAL_RESOURCE, "sha256": file_sha256(staged / METAL_RESOURCE)}:
        raise ValueError("The staged Metal resource does not match its receipt.")
    executable = regular_file(staged / "MossHarness")
    executable_hash = file_sha256(executable)
    if not os.access(executable, os.X_OK) or provenance.get("executableSha256") != executable_hash:
        raise ValueError("The staged worker executable does not match its receipt.")
    if not hex_digest(provenance.get("inputExecutableSha256"), 64):
        raise ValueError("The input executable digest is missing.")
    information = provenance.get("runtimeInfo")
    if not same_json(information, EXPECTED_RUNTIME_INFO):
        raise ValueError("The recorded worker runtime or policy contract is stale.")
    files = package_files(staged)
    if execute_runtime and not same_json(runtime_info(staged, runner=runner), information):
        raise ValueError("The staged worker runtime metadata changed.")
    return {
        "schemaVersion": 1,
        "provenanceSha256": file_sha256(provenance_path),
        "preSignExecutableSha256": executable_hash,
        "runtimeInfo": information,
        "runtimeExecuted": execute_runtime,
        "sourceBinding": provenance["sourceBinding"],
        "files": files,
    }


def signature_metadata(executable, *, runner=subprocess.run):
    result = runner(["codesign", "-d", "--verbose=4", str(executable)],
                    capture_output=True, text=True, check=False, timeout=15)
    if result.returncode:
        raise ValueError("The signed worker identity could not be read.")
    text = result.stdout + result.stderr
    fields = {}
    for name in ("Identifier", "TeamIdentifier", "CDHash"):
        matches = re.findall(r"^" + name + r"=(.+)$", text, re.MULTILINE)
        if len(matches) != 1:
            raise ValueError("The signed worker identity is missing or ambiguous.")
        fields[name] = matches[0].strip()
    if not re.fullmatch(r"[A-Z0-9]{10}", fields["TeamIdentifier"]) or not re.fullmatch(r"(?:[0-9a-fA-F]{40}|[0-9a-fA-F]{64})", fields["CDHash"]):
        raise ValueError("A Developer ID worker signature is required.")
    return fields


def verify_signed_transition(staged, preflight, expected_team, *, runner=subprocess.run):
    staged = Path(staged)
    if preflight.get("schemaVersion") != 1 or not preflight.get("runtimeExecuted"):
        raise ValueError("A verified pre-signing worker package is required.")
    if file_sha256(staged / "RuntimeProvenance.json") != preflight["provenanceSha256"]:
        raise ValueError("Signing must not rewrite the worker runtime provenance.")
    before = preflight["files"]
    after = package_files(staged)
    signature_file = lambda path: "_CodeSignature" in Path(path).parts and Path(path).name == "CodeResources"
    for path, value in before.items():
        if path not in after:
            raise ValueError("A worker package input disappeared during signing.")
        if not value["machO"] and not signature_file(path) and after[path] != value:
            raise ValueError("A non-signature worker resource changed during signing.")
        if value["machO"] and not after[path]["machO"]:
            raise ValueError("A worker executable lost its Mach-O format during signing.")
    if any(path not in before and not signature_file(path) for path in after):
        raise ValueError("An unexpected worker resource appeared during signing.")
    result = runner(["codesign", "--verify", "--strict", str(staged / "MossHarness")],
                    capture_output=True, text=True, check=False, timeout=15)
    if result.returncode:
        raise ValueError("The signed worker verification failed.")
    identity = signature_metadata(staged / "MossHarness", runner=runner)
    if identity["TeamIdentifier"] != expected_team:
        raise ValueError("The worker and application signing teams differ.")
    information = runtime_info(staged, runner=runner)
    if not same_json(information, preflight["runtimeInfo"]):
        raise ValueError("The signed worker runtime contract changed.")
    encoded = json.dumps(after, sort_keys=True, separators=(",", ":")).encode()
    return {
        "schemaVersion": 1, "provenanceSha256": preflight["provenanceSha256"],
        "preSignExecutableSha256": preflight["preSignExecutableSha256"],
        "postSignExecutableSha256": file_sha256(staged / "MossHarness"),
        "signature": identity, "runtimeInfo": information,
        "filesSha256": hashlib.sha256(encoded).hexdigest(),
        "sourceBinding": preflight["sourceBinding"],
        "boundary": "pre-sign source/package verification followed by Developer ID signing; runtime metadata is not full semantic equivalence proof",
    }


def verify_signed_receipt(staged, receipt, expected_team, *, runner=subprocess.run):
    """Verify the preserved signed package without treating pre-sign hashes as current."""
    staged = Path(staged)
    if not isinstance(receipt, dict) or type(receipt.get("schemaVersion")) is not int or receipt["schemaVersion"] != 1:
        raise ValueError("A signed worker package receipt is required.")
    provenance = read_json(staged / "RuntimeProvenance.json")
    if file_sha256(staged / "RuntimeProvenance.json") != receipt.get("provenanceSha256"):
        raise ValueError("The preserved worker provenance no longer matches its signed receipt.")
    if provenance.get("executableSha256") != receipt.get("preSignExecutableSha256"):
        raise ValueError("The signed receipt is not linked to the packaged executable.")
    if (provenance.get("signatureStage") != "workerAdHocBeforeDeveloperId"
            or provenance.get("sourceBinding") != "recordedAtPackagingNotEmbeddedInExecutable"
            or receipt.get("sourceBinding") != provenance.get("sourceBinding")):
        raise ValueError("The signed receipt has an unsupported source or signature binding.")
    files = package_files(staged)
    encoded = json.dumps(files, sort_keys=True, separators=(",", ":")).encode()
    if hashlib.sha256(encoded).hexdigest() != receipt.get("filesSha256") or file_sha256(staged / "MossHarness") != receipt.get("postSignExecutableSha256"):
        raise ValueError("The signed worker package differs from its preserved receipt.")
    result = runner(["codesign", "--verify", "--strict", str(staged / "MossHarness")],
                    capture_output=True, text=True, check=False, timeout=15)
    if result.returncode:
        raise ValueError("The preserved worker signature verification failed.")
    identity = signature_metadata(staged / "MossHarness", runner=runner)
    if identity["TeamIdentifier"] != expected_team or not same_json(identity, receipt.get("signature")):
        raise ValueError("The preserved worker signing identity differs from its receipt.")
    information = runtime_info(staged, runner=runner)
    if not same_json(information, receipt.get("runtimeInfo")) or not same_json(information, provenance.get("runtimeInfo")):
        raise ValueError("The preserved worker runtime metadata differs from its receipt.")
    return receipt


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
    staged = subparsers.add_parser("verify-staged")
    staged.add_argument("staged", type=Path)
    staged.add_argument("--optional", action="store_true")
    staged.add_argument("--runtime-info", action="store_true")
    arguments = parser.parse_args()
    try:
        if arguments.operation == "preflight":
            verify_inputs(arguments.products, arguments.checkouts, arguments.patch_receipt)
        elif arguments.operation == "finalize":
            finalize(arguments.products, arguments.checkouts, arguments.staged, arguments.patch_receipt)
        else:
            if arguments.optional and not arguments.staged.exists() and not arguments.staged.is_symlink():
                print("MOSS worker absent: optional transcription is unavailable.")
                return
            verified = verify_staged(arguments.staged, execute_runtime=arguments.runtime_info)
            print(json.dumps({key: value for key, value in verified.items() if key != "files"}, sort_keys=True))
    except (ValueError, OSError, KeyError, TypeError) as error:
        parser.exit(2, str(error) + "\n")


if __name__ == "__main__":
    main()
