#!/usr/bin/env python3
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile


REPO = Path(__file__).resolve().parents[2]
WORK_ROOT = REPO / ".work"
PATCHES = Path(__file__).with_name("patches")
PINNED_COMMIT = "d302a5c6080d2bb97bae38c7418f82abb76013b6"
SOURCE_FILE = "Sources/MLXAudioSTT/Models/MossTranscribeDiarize/MossTranscribeDiarize.swift"
SOURCE_SHA256 = "14f9aeb1ed2ac1ae1db3b205ae84cc10cba41b9939a4af241a39b5371e4121b7"


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def owned_path(path, work_root):
    work = Path(os.path.abspath(work_root))
    candidate = Path(os.path.abspath(path))
    if work.is_symlink() or not work.is_dir():
        raise ValueError("The repository .work directory must be a real directory.")
    if not candidate.is_relative_to(work) or not candidate.resolve().is_relative_to(work.resolve()):
        raise ValueError("Only a task-owned path beneath the repository .work directory is allowed.")
    current = candidate
    while current != work:
        if current.is_symlink():
            raise ValueError("Symlinks are not allowed in a task-owned patch path.")
        current = current.parent
    return candidate


def git(checkout, *arguments):
    result = subprocess.run(
        ["git", "-C", str(checkout), *arguments], capture_output=True, text=True, check=False,
    )
    if result.returncode:
        raise ValueError("Dependency Git validation or patch application failed.")
    return result.stdout


def load_contract():
    manifest = json.loads((PATCHES / "manifest.json").read_text())
    if (manifest.get("schemaVersion") != 1
        or manifest.get("upstreamCommit") != PINNED_COMMIT
        or manifest.get("sourceFile") != SOURCE_FILE
        or manifest.get("unpatchedSha256") != SOURCE_SHA256
        or manifest.get("runtime") != "mlx-audio-swift-v0.1.3+nero-moss-whole-v2"
        or manifest.get("metadataSchemaVersion") != 2
        or manifest.get("eosPolicy") != "model-generation-config+tokenizer-v1"
        or manifest.get("samplingPolicy") != "greedy-argmax-v1"
        or manifest.get("modelRevision") != "704aa4a9c304e8520be88901e0d1960158ef5b15"
        or manifest.get("generationConfigSha256") != "e53a4b3ce4f944230cf1ca8fed0c42f4ff0d8c1443eaf98b5315d987334dd9e4"
        or manifest.get("tokenizerConfigSha256") != "61d04c96104177240688396655ae3f7cf38ce2ea036db867a1d2b6883e27c3d5"):
        raise ValueError("The patch manifest does not match the pinned dependency contract.")
    patch_name = manifest.get("patchFile")
    if not isinstance(patch_name, str) or Path(patch_name).name != patch_name:
        raise ValueError("Invalid patch filename.")
    patch = PATCHES / patch_name
    text = patch.read_bytes()
    if sha256(text) != manifest.get("patchSha256"):
        raise ValueError("The patch digest does not match its manifest.")
    headers = [line for line in text.decode().splitlines() if line.startswith(("--- ", "+++ "))]
    if headers != ["--- a/" + SOURCE_FILE, "+++ b/" + SOURCE_FILE]:
        raise ValueError("The patch must change only the pinned MOSS source file.")
    return manifest, patch


def apply_runtime_patch(checkout, task_root, receipt, *, work_root=WORK_ROOT,
                        contract=None, patch=None, git_command=git):
    task = owned_path(task_root, work_root)
    checkout = owned_path(checkout, work_root)
    receipt = owned_path(receipt, work_root)
    if task == Path(os.path.abspath(work_root)) or not task.is_dir() or not checkout.is_dir() or not checkout.is_relative_to(task):
        raise ValueError("The dependency checkout must belong to the selected task directory.")
    if not receipt.is_relative_to(task) or receipt.is_relative_to(checkout):
        raise ValueError("The receipt must be inside the selected task, outside the dependency checkout.")
    if contract is None:
        contract, patch = load_contract()
    if patch is None or sha256(Path(patch).read_bytes()) != contract["patchSha256"]:
        raise ValueError("The patch digest does not match its contract.")
    source = owned_path(checkout / contract["sourceFile"], work_root)
    if not source.is_file() or source.stat().st_mode & 0o111:
        raise ValueError("The pinned MOSS source file is missing.")
    if Path(git_command(checkout, "rev-parse", "--show-toplevel").strip()).resolve() != checkout.resolve():
        raise ValueError("Expected a dependency Git checkout root.")
    if git_command(checkout, "rev-parse", "HEAD").strip() != contract["upstreamCommit"]:
        raise ValueError("The dependency commit does not match the pinned release.")
    digest = sha256(source.read_bytes())
    if digest not in (contract["unpatchedSha256"], contract["patchedSha256"]):
        raise ValueError("The MOSS source has an unknown modification; refusing to patch it.")
    expected_status = "" if digest == contract["unpatchedSha256"] else " M " + contract["sourceFile"] + "\n"
    if git_command(checkout, "status", "--porcelain=v1", "--untracked-files=all", "--ignore-submodules=none") != expected_status:
        raise ValueError("The dependency has unexpected staged, modified, or untracked files.")
    provenance = dict(contract)
    provenance["upstreamTree"] = git_command(checkout, "rev-parse", "HEAD^{tree}").strip()
    if receipt.exists():
        if not receipt.is_file() or receipt.stat().st_size > 65536 or json.loads(receipt.read_text()) != provenance:
            raise ValueError("An existing receipt does not match this patch provenance.")
    action = "alreadyApplied"
    if digest == contract["unpatchedSha256"]:
        git_command(checkout, "apply", "--check", "--", str(Path(patch).resolve()))
        git_command(checkout, "apply", "--", str(Path(patch).resolve()))
        action = "applied"
    if sha256(source.read_bytes()) != contract["patchedSha256"]:
        raise ValueError("Patched source digest mismatch; do not build this dependency checkout.")
    if git_command(checkout, "status", "--porcelain=v1", "--untracked-files=all", "--ignore-submodules=none") != " M " + contract["sourceFile"] + "\n":
        raise ValueError("Unexpected dependency changes after patch application; do not build.")
    if not receipt.exists():
        receipt.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.NamedTemporaryFile(mode="w", dir=receipt.parent, delete=False) as output:
            temporary = Path(output.name)
            json.dump(provenance, output, indent=2)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        try:
            os.link(temporary, receipt)
        finally:
            temporary.unlink()
    return {"action": action, "runtime": contract["runtime"], "patchedSha256": contract["patchedSha256"]}


def main():
    parser = argparse.ArgumentParser(description="Verify and apply the pinned Nero MOSS runtime patch in a task-owned cache.")
    parser.add_argument("--checkout", type=Path, required=True)
    parser.add_argument("--task-root", type=Path, required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    arguments = parser.parse_args()
    try:
        result = apply_runtime_patch(arguments.checkout, arguments.task_root, arguments.receipt)
    except (ValueError, OSError, KeyError, json.JSONDecodeError) as error:
        parser.exit(2, str(error) + "\n")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
