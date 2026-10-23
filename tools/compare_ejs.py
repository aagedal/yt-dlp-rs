#!/usr/bin/env python3
"""Compare saved EJS vectors through pinned upstream's payload constructor.

Development only: no live player requests, package downloads, or extraction.
The only ignored field is upstream's optional preprocessed_player source.
Use --check-rust to run Rust's saved-vector test against the same Deno path.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[1]
PINNED_UPSTREAM = "6ccd977303412151ee23352c6257dd658ffc1987"
PINNED_DENO = "2.9.7"
DENO_ARGS = [
    "run", "--ext=js", "--no-code-cache", "--no-prompt", "--no-remote",
    "--no-lock", "--node-modules-dir=none", "--no-config", "--no-npm",
    "--cached-only", "-",
]
UPSTREAM_WORKER = r'''
import json, socket, sys
sys.stdout.reconfigure(encoding="utf-8")
from pathlib import Path
from types import SimpleNamespace

def forbidden(*args, **kwargs):
    raise AssertionError('Network access forbidden in saved EJS comparison')

class ForbiddenSocket(socket.socket):
    def __new__(cls, *args, **kwargs):
        return forbidden()

socket.create_connection = forbidden
socket.socket = ForbiddenSocket
sys.path.insert(0, sys.argv[1])
from yt_dlp.extractor.youtube.jsc._builtin.ejs import EJSBaseJCP
from yt_dlp.extractor.youtube.jsc.provider import (
    JsChallengeRequest, JsChallengeType, NChallengeInput, SigChallengeInput,
)
root = Path(sys.argv[2])
directory = root / 'tests/fixtures/youtube/challenges'
fixture = json.loads((directory / 'expected.json').read_text(encoding="utf-8"))
requests = []
for request in fixture['requests']:
    kind = JsChallengeType(request['type'])
    input_type = NChallengeInput if kind is JsChallengeType.N else SigChallengeInput
    requests.append(JsChallengeRequest(
        kind, input_type(fixture['player_url'], list(request['challenges'])), 'saved-player'))
provider = SimpleNamespace(
    _lib_script=SimpleNamespace(code=(root / 'vendor/ejs/yt.solver.lib.js').read_text(encoding="utf-8")),
    _core_script=SimpleNamespace(code=(root / 'vendor/ejs/yt.solver.core.js').read_text(encoding="utf-8")),
)
player = (directory / fixture['player_file']).read_text(encoding="utf-8")
sys.stdout.write(EJSBaseJCP._construct_stdin(provider, player, False, requests))
'''


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON key")
        result[key] = value
    return result


def parse_output(output):
    return json.loads(output, object_pairs_hook=unique_object)


def compare(actual, fixture):
    """Compare every envelope/result field, ignoring only prepared source."""
    if not isinstance(actual, dict):
        raise ValueError("solver envelope must be an object")
    normalized = dict(actual)
    if "preprocessed_player" in normalized:
        if not isinstance(normalized.pop("preprocessed_player"), str):
            raise ValueError("preprocessed_player must be a string")
    expected = {
        "type": "result",
        "responses": [
            {"type": "result", "data": request["challenges"]}
            for request in fixture["requests"]
        ],
    }
    if normalized != expected:
        raise ValueError("saved EJS challenge results differ from exact upstream vectors")
    return normalized


def runtime_environment(directory):
    environment = {
        "DENO_DIR": str(directory), "DENO_NO_UPDATE_CHECK": "1",
        "DENO_NO_PACKAGE_JSON": "1", "NO_COLOR": "1",
    }
    if sys.platform == "win32":
        for name in ("SystemRoot", "WINDIR"):
            if name in os.environ:
                environment[name] = os.environ[name]
    return environment


def run_deno(deno, script):
    with tempfile.TemporaryDirectory(prefix="yt-dlp-rs-ejs-") as directory:
        environment = runtime_environment(directory)
        version = subprocess.run(
            [str(deno), "--version"], capture_output=True, text=True, encoding="utf-8", check=True,
            timeout=15, env=environment, cwd=directory,
        )
        first_line = version.stdout.splitlines()[0] if version.stdout else ""
        if version.stderr or not (
            first_line == f"deno {PINNED_DENO} (stable, release)"
            or (first_line.startswith(f"deno {PINNED_DENO} (stable, release, ")
                and first_line.endswith(")"))
        ):
            raise ValueError(f"comparison requires pinned Deno {PINNED_DENO}")
        completed = subprocess.run(
            [str(deno), *DENO_ARGS], input=script, capture_output=True, text=True, encoding="utf-8",
            check=True, timeout=15, env=environment, cwd=directory,
        )
        if completed.stderr:
            raise ValueError("Deno emitted stderr during saved EJS comparison")
        return parse_output(completed.stdout)


def verify_inputs(root):
    manifest = json.loads((root / "vendor/ejs/manifest.json").read_text(encoding="utf-8"))
    for name, entry in manifest["files"].items():
        digest = hashlib.sha3_512((root / "vendor/ejs" / name).read_bytes()).hexdigest()
        if digest != entry["sha3_512"]:
            raise ValueError(f"vendored EJS asset hash mismatch: {name}")
    directory = root / "tests/fixtures/youtube/challenges"
    fixture = json.loads((directory / "expected.json").read_text(encoding="utf-8"))
    player_hash = hashlib.sha3_512((directory / fixture["player_file"]).read_bytes()).hexdigest()
    if player_hash != fixture["player_sha3_512"]:
        raise ValueError("saved player hash mismatch")
    if fixture["upstream_commit"] != PINNED_UPSTREAM:
        raise ValueError("saved vectors have an unexpected upstream revision")
    return fixture


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--upstream", required=True, type=Path)
    parser.add_argument("--deno", required=True, type=Path)
    parser.add_argument("--check-rust", action="store_true")
    args = parser.parse_args()
    try:
        upstream = args.upstream.resolve(strict=True)
        deno = args.deno.resolve(strict=True)
        revision = subprocess.run(
            ["git", "-C", str(upstream), "rev-parse", "HEAD"],
            capture_output=True, text=True, encoding="utf-8", timeout=15, check=True,
        ).stdout.strip()
        if revision != PINNED_UPSTREAM:
            raise ValueError(f"--upstream must be checked out at {PINNED_UPSTREAM}")
        fixture = verify_inputs(ROOT)
        constructed = subprocess.run(
            [sys.executable, "-B", "-c", UPSTREAM_WORKER, str(upstream), str(ROOT)],
            capture_output=True, text=True, encoding="utf-8", timeout=30, check=True,
        )
        if constructed.stderr:
            raise ValueError("upstream payload constructor emitted stderr")
        compare(run_deno(deno, constructed.stdout), fixture)
        if args.check_rust:
            environment = dict(os.environ, YT_DLP_RS_TEST_DENO=str(deno))
            subprocess.run(
                ["cargo", "test", "--locked", "--lib",
                 "extractor::youtube::challenge::runtime::tests::real_saved_player_vectors",
                 "--", "--exact"],
                cwd=ROOT, env=environment, timeout=300, check=True,
            )
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        if isinstance(error, subprocess.CalledProcessError):
            diagnostic = (error.stderr or "")[:4096]
            print(f"comparison subprocess failed (exit {error.returncode}): {diagnostic!r}",
                  file=sys.stderr)
        else:
            print(f"comparison failed: {error}", file=sys.stderr)
        return 1
    print("Saved n/signature vectors match pinned upstream EJS output exactly")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
