import contextlib
import copy
import io
import json
import math
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_filters import FIXTURE, PINNED_UPSTREAM, compare, main


class FormatFilterComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
        self.results = [{"name": case["name"], "matching_ids": case["expected_ids"]}
                        for case in self.fixture["cases"]]

    def test_exact_case_and_match_order_are_compared_without_mutation(self):
        before = copy.deepcopy((self.results, self.fixture))
        self.assertEqual(compare(self.results, self.fixture["cases"]), self.results)
        self.assertEqual((self.results, self.fixture), before)
        changes = [self.results[::-1], self.results[:-1]]
        changed = copy.deepcopy(self.results)
        changed[0]["matching_ids"] = changed[0]["matching_ids"][::-1] + ["unexpected"]
        changes.append(changed)
        for result in changes:
            with self.subTest(result=result), self.assertRaisesRegex(ValueError, "predicate match mismatch"):
                compare(result, self.fixture["cases"])

    def test_fixture_covers_six_numeric_operators_and_missing_values(self):
        cases = {case["predicate"]: case["expected_ids"] for case in self.fixture["cases"]}
        for op in ("<", "<=", ">", ">=", "=", "!="):
            self.assertIn(f"height {op} 720", cases)
            self.assertIn(f"height {op}? 720", cases)
            self.assertNotIn("missing", cases[f"height {op} 720"])
            self.assertIn("missing", cases[f"height {op}? 720"])
        self.assertEqual(cases["height = 720"], ["mid"])
        self.assertEqual(cases["height != 720"], ["low", "high"])
        self.assertEqual(cases["filesize<1M"], ["low", "mid", "high"])
        self.assertEqual(cases["filesize<1MiB"], ["low", "mid", "high", "decimal"])
        self.assertEqual(cases["filesize=0.5kB"], [])
        self.assertEqual(cases["filesize=0.5KB"], ["low"])
        self.assertEqual(cases["filesize=1KB"], ["mid"])
        self.assertEqual(cases["filesize=1kB"], [])
        self.assertEqual(cases["filesize=9007199254740992"], ["limit"])
        self.assertEqual(cases["filesize=9007199254740991"], ["near-limit"])
        self.assertTrue(all(fmt.get("filesize") is None or fmt["filesize"] <= 2**53
                            for fmt in self.fixture["formats"]))

    def test_fixture_preserves_text_negation_quotes_escapes_and_delimiters(self):
        cases = {case["predicate"]: case["expected_ids"] for case in self.fixture["cases"]}
        for op in ("=", "^=", "$=", "*=", "!=", "!^=", "!$=", "!*="):
            self.assertTrue(any(predicate.startswith("language" + op) for predicate in cases), op)
            self.assertTrue(any(predicate.startswith("language" + op + "?") for predicate in cases), op)
        self.assertEqual(cases['format_note*="[safe]+audio"'], ["low"])
        self.assertEqual(cases['format_note*="[720p]/+ fallback"'], ["mid"])
        self.assertEqual(cases[r'format_note*="\"quote\""'], ["mid"])
        self.assertEqual(cases[r"format_note^='Director\'s'"], ["high"])
        self.assertEqual(cases[r'format_note*="\\ surround"'], ["high"])
        self.assertEqual(cases[" language ! ^= ? 'en' "],
                         ["high", "binary", "missing", "limit", "near-limit", "1", "zero"])
        self.assertFalse(any("~=" in predicate for predicate in cases))
        allowed = {"format_id", "url", "ext", "width", "height", "filesize", "tbr", "abr",
                   "language", "format_note", "audio_group"}
        self.assertTrue(all(set(fmt) <= allowed for fmt in self.fixture["formats"]))
        self.assertEqual(self.fixture["upstream_commit"], PINNED_UPSTREAM)

    def test_numeric_looking_text_uses_upstream_numeric_first_branch(self):
        cases = {case["predicate"]: case["expected_ids"] for case in self.fixture["cases"]}
        all_ids = [fmt["format_id"] for fmt in self.fixture["formats"]]
        for field, literal in (("format_id", "1"), ("ext", "123"), ("language", "123")):
            self.assertEqual(cases[f"{field}={literal}"], [])
            self.assertEqual(cases[f'{field}="{literal}"'], ["1"])
            self.assertIn("1", cases[f"{field}!={literal}"])
            self.assertNotIn("1", cases[f'{field}!="{literal}"'])
        self.assertEqual(cases["format_id!=1"], all_ids)
        self.assertEqual(cases["format_id=?1"], [])
        self.assertIn("missing", cases["language=?123"])
        self.assertNotIn("1", cases["language=?123"])
        self.assertIn("missing", cases['language=?"123"'])
        self.assertIn("1", cases['language=?"123"'])

    def test_positive_and_negative_json_zero_rates_match_numeric_zero(self):
        cases = {case["predicate"]: case["expected_ids"] for case in self.fixture["cases"]}
        by_id = {fmt["format_id"]: fmt for fmt in self.fixture["formats"]}
        self.assertEqual(math.copysign(1, by_id["1"]["abr"]), -1)
        self.assertEqual(math.copysign(1, by_id["zero"]["tbr"]), -1)
        for field in ("abr", "tbr"):
            self.assertEqual(cases[f"{field}=0"], ["1", "zero"])
            self.assertEqual(cases[f"{field}<=0"], ["1", "zero"])
            self.assertEqual(cases[f"{field}<0"], [])
            self.assertNotIn("1", cases[f"{field}!=0"])
            self.assertNotIn("zero", cases[f"{field}!=0"])
            self.assertIn("missing", cases[f"{field}=?0"])

    def test_command_uses_pinned_actual_worker_with_exact_fixture(self):
        with tempfile.TemporaryDirectory() as directory:
            upstream = Path(directory)
            module = upstream / "yt_dlp/YoutubeDL.py"
            module.parent.mkdir(parents=True)
            module.touch()
            results = [subprocess.CompletedProcess([], 0, PINNED_UPSTREAM + "\n", ""),
                       subprocess.CompletedProcess([], 0, json.dumps(self.results), "")]
            with mock.patch("sys.argv", ["compare_filters.py", "--upstream", directory]), \
                    mock.patch("compare_filters.subprocess.run", side_effect=results) as run, \
                    contextlib.redirect_stdout(io.StringIO()) as stdout:
                self.assertEqual(main(), 0)
            self.assertEqual(run.call_count, 2)
            self.assertEqual(run.call_args_list[0].args[0],
                             ["git", "-C", str(upstream.resolve()), "rev-parse", "HEAD"])
            self.assertEqual(json.loads(run.call_args.kwargs["input"]), self.fixture)
            self.assertEqual(run.call_args.kwargs["encoding"], "utf-8")
            self.assertIn(f"{len(self.fixture['cases'])} saved format predicates", stdout.getvalue())

    def test_wrong_checkout_revision_fails_before_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            upstream = Path(directory)
            module = upstream / "yt_dlp/YoutubeDL.py"
            module.parent.mkdir(parents=True)
            module.touch()
            with mock.patch("sys.argv", ["compare_filters.py", "--upstream", directory]), \
                    mock.patch("compare_filters.subprocess.run", return_value=subprocess.CompletedProcess(
                        [], 0, "0" * 40, "")) as run, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                self.assertEqual(main(), 1)
            self.assertEqual(run.call_count, 1)
            self.assertIn(PINNED_UPSTREAM, stderr.getvalue())

    def test_wrong_fixture_revision_fails_before_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            upstream = Path(directory)
            module = upstream / "yt_dlp/YoutubeDL.py"
            module.parent.mkdir(parents=True)
            module.touch()
            fixture_path = upstream / "wrong-fixture.json"
            self.fixture["upstream_commit"] = "0" * 40
            fixture_path.write_text(json.dumps(self.fixture), encoding="utf-8")
            with mock.patch("sys.argv", ["compare_filters.py", "--upstream", directory]), \
                    mock.patch("compare_filters.FIXTURE", fixture_path), \
                    mock.patch("compare_filters.subprocess.run", return_value=subprocess.CompletedProcess(
                        [], 0, PINNED_UPSTREAM, "")) as run, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                self.assertEqual(main(), 1)
            self.assertEqual(run.call_count, 1)
            self.assertIn("fixture does not match", stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
