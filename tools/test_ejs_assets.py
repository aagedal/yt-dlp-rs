"""Offline integrity checks for pinned EJS code and saved player vectors."""

import hashlib
import json
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
UPSTREAM_COMMIT = "897341241f2bfb2439fe0367f4c7c9ccd29a3828"
ASSET_HASHES = {
    "yt.solver.lib.js": "1ee3753a8222fc855f5c39db30a9ccbb7967dbe1fb810e86dc9a89aa073a0907f294c720e9b65427d560a35aa1ce6af19ef854d9126a05ca00afe03f72047733",
    "yt.solver.core.js": "c163a6f376db6ce3da47d516a28a8f2a0554ae95c58dc766f0a6e2b3894f2cef1ee07fa84beb442fa471aac4f300985added1657c7c94c4d1cfefe68920ab599",
}
PLAYER_HASH = "1fd297295bdaca4b870512d9c2d8246dce31450c6fbb059a5ce081deafbe7c10673c9ab73df6e002bb5a6070b0face3872f4463c4fb57f362cfe6cb62527c2f8"
VECTORS_HASH = "c3ebf2964da045159995865b81384e6fc679e67a02c81bc707f625b5ff92793f64d424e99989e443cfe7a73f8c93940c93eb1fa2d39b72bb9927315abc159383"


class EjsAssetsTests(unittest.TestCase):
    def test_pinned_solver_assets_and_license(self):
        directory = ROOT / "vendor/ejs"
        manifest = json.loads((directory / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest["schema_version"], 1)
        self.assertEqual(manifest["version"], "0.8.0")
        self.assertEqual(manifest["upstream_commit"], UPSTREAM_COMMIT)
        self.assertEqual(set(manifest["files"]), {*ASSET_HASHES, "LICENSE"})
        self.assertIn(UPSTREAM_COMMIT, manifest["verification_source"])
        for name, metadata in manifest["files"].items():
            with self.subTest(asset=name):
                content = (directory / name).read_bytes()
                digest = hashlib.sha3_512(content).hexdigest()
                self.assertEqual(digest, metadata["sha3_512"])
                if name in ASSET_HASHES:
                    self.assertEqual(digest, ASSET_HASHES[name])
                    self.assertEqual(
                        metadata["url"],
                        f"https://github.com/yt-dlp/ejs/releases/download/0.8.0/{name}",
                    )
                    self.assertIn(b"SPDX-License-Identifier: Unlicense", content)
        library = (directory / "yt.solver.lib.js").read_text(encoding="utf-8")
        for notice in ("Name: meriyah", "ISC License", "Name: astring", "License: MIT"):
            self.assertTrue(notice in library, f"Missing bundled dependency notice: {notice}")
        self.assertIn("public domain", (directory / "LICENSE").read_text(encoding="utf-8"))

    def test_saved_player_and_exact_vector_schema(self):
        directory = ROOT / "tests/fixtures/youtube/challenges"
        vector_bytes = (directory / "expected.json").read_bytes()
        self.assertEqual(hashlib.sha3_512(vector_bytes).hexdigest(), VECTORS_HASH)
        fixture = json.loads(vector_bytes)
        self.assertEqual(fixture["schema_version"], 1)
        self.assertEqual(fixture["upstream_commit"], UPSTREAM_COMMIT)
        self.assertEqual(fixture["player_id"], "74edf1a3")
        self.assertEqual(fixture["player_file"], "player-74edf1a3.js")
        self.assertEqual(fixture["player_sha3_512"], PLAYER_HASH)
        self.assertEqual(
            hashlib.sha3_512((directory / fixture["player_file"]).read_bytes()).hexdigest(),
            PLAYER_HASH,
        )
        self.assertEqual(
            fixture["player_url"],
            "https://www.youtube.com/s/player/74edf1a3/player_ias.vflset/en_US/base.js",
        )
        self.assertIn(UPSTREAM_COMMIT, fixture["vectors_source"])
        requests = fixture["requests"]
        self.assertEqual([request["type"] for request in requests], ["n", "sig"])
        self.assertEqual([len(request["challenges"]) for request in requests], [3, 2])
        for request in requests:
            self.assertEqual(set(request), {"type", "challenges"})
            self.assertIsInstance(request["challenges"], dict)
            for challenge, solution in request["challenges"].items():
                self.assertIsInstance(challenge, str)
                self.assertIsInstance(solution, str)
                self.assertTrue(challenge)
                self.assertTrue(solution)
        signature_vectors = requests[1]["challenges"]
        self.assertTrue(any("\x00" in challenge for challenge in signature_vectors))
        self.assertTrue(any("\x00" in solution for solution in signature_vectors.values()))
        self.assertIn("eabGF/ps%UK=uWHXGh6FR4", requests[0]["challenges"])


if __name__ == "__main__":
    unittest.main()
