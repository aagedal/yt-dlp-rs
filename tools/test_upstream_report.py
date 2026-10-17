import json
from pathlib import Path
import subprocess
import tempfile
import unittest

from upstream_report import build_report, matches


class UpstreamReportTest(unittest.TestCase):
    def test_exact_and_directory_mapping(self):
        self.assertTrue(matches("yt_dlp/extractor/youtube/video.py", ["yt_dlp/extractor/youtube/"]))
        self.assertFalse(matches("yt_dlp/extractor/generic.py.extra", ["yt_dlp/extractor/generic.py"]))

    def test_real_git_changes_include_deletions_and_unmapped_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)

            def git(*args):
                return subprocess.check_output(["git", "-C", directory, *args], stderr=subprocess.DEVNULL).decode().strip()

            git("init")
            git("config", "user.name", "Fixture")
            git("config", "user.email", "fixture@example.invalid")
            (root / "known.py").write_text("before\n")
            git("add", ".")
            git("commit", "-m", "baseline")
            baseline = git("rev-parse", "HEAD")
            (root / "known.py").unlink()
            (root / "new.py").write_text("after\n")
            git("add", "-A")
            git("commit", "-m", "upstream change")
            manifest = {
                "repository": "fixture", "baseline": baseline,
                "components": [{"name": "known", "upstream": ["known.py"], "rust": ["src/lib.rs"], "status": "partial"}],
            }
            report = build_report(root, manifest)
            self.assertEqual(report["changed_files"], ["known.py", "new.py"])
            self.assertEqual(report["unmapped_files"], ["new.py"])
            self.assertEqual(report["affected_components"][0]["changed_files"], ["known.py"])
            self.assertEqual(report["commits"][0]["subject"], "upstream change")
            self.assertEqual(build_report(root, manifest, target=baseline)["changed_files"], [])
            with self.assertRaises(subprocess.CalledProcessError):
                build_report(root, manifest, target=baseline, base="HEAD")
            # The report is directly consumable by an agent or CI job.
            self.assertEqual(json.loads(json.dumps(report)), report)


if __name__ == "__main__":
    unittest.main()
