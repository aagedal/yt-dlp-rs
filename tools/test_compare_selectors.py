import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_selectors import FIXTURE, PINNED_UPSTREAM, compare, main


class SelectorExpressionComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
        self.result = {
            "matches": [{"name": case["name"], "selected_ids": case["expected_ids"]}
                        for case in self.fixture["cases"]],
            "syntax_errors": [case["name"] for case in self.fixture["invalid_cases"]],
            "index_limit_errors": [case["selector"] for case in self.fixture["excluded_index_cases"]],
        }

    def test_exact_output_order_and_pair_order_are_compared_without_mutation(self):
        before = copy.deepcopy((self.fixture, self.result))
        self.assertEqual(compare(self.result, self.fixture), self.result)
        self.assertEqual((self.fixture, self.result), before)
        changes = []
        changed = copy.deepcopy(self.result)
        changed["matches"] = changed["matches"][::-1]
        changes.append(changed)
        changed = copy.deepcopy(self.result)
        pair = next(case for case in changed["matches"] if len(case["selected_ids"]) == 2)
        pair["selected_ids"].reverse()
        changes.append(changed)
        changed = copy.deepcopy(self.result)
        changed["syntax_errors"].pop()
        changes.append(changed)
        for result in changes:
            with self.subTest(result=result), self.assertRaisesRegex(ValueError, "selector expression mismatch"):
                compare(result, self.fixture)

    def test_group_filters_apply_before_children_and_plus_precedes_slash(self):
        cases = {case["selector"]: case["expected_ids"] for case in self.fixture["cases"]}
        self.assertEqual(cases["bv+ba/b"], ["video-high", "audio-high"])
        self.assertEqual(cases["bv/(ba+b)"], ["video-high"])
        self.assertEqual(cases["(bv+ba/b)[height<=720]"], ["mux-high"])
        self.assertEqual(cases["bv+ba[height<=720]"], [])
        self.assertEqual(cases["bv[height<=720]+ba"], ["video-mid", "audio-high"])
        self.assertEqual(cases["(bv+ba)[height<=?720]"], ["video-mid", "audio-high"])
        self.assertEqual(cases["((missing/bv)+absent)/b"], ["mux-high"])
        self.assertEqual(cases["(video-low/video-high)+audio-low"], ["video-low", "audio-low"])
        self.assertEqual(cases["(missing/absent)"], [])

    def test_aliases_bare_filters_and_explicit_native_kind_mapping(self):
        cases = {case["selector"]: case["expected_ids"] for case in self.fixture["cases"]}
        for base, expected in (("best", "mux-high"), ("b", "mux-high"),
                               ("worst", "mux-low"), ("w", "mux-low")):
            self.assertEqual(cases[base], [expected])
            for suffix in ("video", "v"):
                self.assertEqual(cases[base + suffix], ["video-high" if base in ("best", "b") else "video-low"])
            for suffix in ("audio", "a"):
                self.assertEqual(cases[base + suffix], ["audio-high" if base in ("best", "b") else "audio-low"])
        self.assertEqual(cases["[height<=720]"], ["mux-high"])
        self.assertEqual(cases["([height<=360])"], ["mux-low"])
        self.assertEqual(cases["([height<=720]/b)[tbr<600]"], ["mux-low"])
        codecs = {"video_only": (True, False), "audio_only": (False, True), "standalone": (True, True)}
        formats = self.fixture["info"]["formats"]
        for fmt in formats:
            self.assertEqual((fmt["vcodec"] != "none", fmt["acodec"] != "none"), codecs[fmt["stream_kind"]])
        for kind in codecs:
            typed = [fmt for fmt in formats if fmt["stream_kind"] == kind]
            scores = [(fmt.get("height", 0), fmt["tbr"]) for fmt in typed]
            self.assertEqual(scores, sorted(scores))
        self.assertEqual(self.fixture["upstream_commit"], PINNED_UPSTREAM)
        self.assertTrue(all(case["selector"].startswith("b/") for case in self.fixture["invalid_cases"]))

    def test_indexed_aliases_choose_nth_in_supplied_order_after_filters(self):
        cases = {case["selector"]: case["expected_ids"] for case in self.fixture["cases"]}
        for base in ("best", "worst", "b", "w"):
            best = base in ("best", "b")
            for suffix in ("", "video", "v", "audio", "a"):
                ranked = (["mux-high", "mux-low"] if not suffix else
                          ["video-high", "video-mid", "video-low"] if suffix in ("video", "v") else
                          ["audio-high", "audio-low"])
                if not best:
                    ranked = ranked[::-1]
                for index in (1, 2, 3, 9):
                    self.assertEqual(cases[f"{base}{suffix}.{index}"],
                                     ranked[index - 1:index] if index <= len(ranked) else [])
        self.assertEqual(cases["bv.2[height<=720]"], ["video-low"])
        self.assertEqual(cases["bv.2[height>=720]"], ["video-mid"])
        self.assertEqual(cases["bv.2[height<720]"], [])
        self.assertEqual(cases["(bv.2+ba.2)/b"], ["video-mid", "audio-low"])
        self.assertEqual(cases["ba.2+bv.2"], ["audio-low", "video-mid"])
        self.assertEqual(cases["(bv.2+ba.2/b)[height<=?720]"], ["video-low", "audio-low"])
        self.assertEqual(cases["(bv.2+ba.2/b)[height<=720]"], ["mux-high"])

    def test_invalid_index_spellings_are_literal_ids_and_reference_limit_errors_are_separate(self):
        cases = {case["selector"]: case["expected_ids"] for case in self.fixture["cases"]}
        for selector in ("bv.0", "bv.01", "bv.x"):
            self.assertEqual(cases[selector], [])
        self.assertEqual(cases["bv.0/bv"], ["video-high"])
        self.assertEqual(cases["bv.01/bv.2"], ["video-mid"])
        self.assertEqual(cases["bv.x/bv.3"], ["video-low"])
        self.assertEqual(cases["b.1000000000000000000"], [])
        self.assertEqual(cases["b.9223372036854775807"], [])
        excluded = self.fixture["excluded_index_cases"]
        self.assertEqual(len(excluded), 4)
        for case in excluded:
            self.assertNotIn(case["selector"], cases)
            self.assertIn("ValueError", case["upstream_behavior"])
            self.assertIn("parse error", case["native_policy"])
        changed = copy.deepcopy(self.result)
        changed["index_limit_errors"].pop()
        with self.assertRaisesRegex(ValueError, "selector expression mismatch"):
            compare(changed, self.fixture)

    def test_command_uses_actual_worker_with_exact_fixture_at_pinned_revision(self):
        with tempfile.TemporaryDirectory() as directory:
            upstream = Path(directory)
            module = upstream / "yt_dlp/YoutubeDL.py"
            module.parent.mkdir(parents=True)
            module.touch()
            results = [subprocess.CompletedProcess([], 0, PINNED_UPSTREAM + "\n", ""),
                       subprocess.CompletedProcess([], 0, json.dumps(self.result), "")]
            with mock.patch("sys.argv", ["compare_selectors.py", "--upstream", directory]), \
                    mock.patch("compare_selectors.subprocess.run", side_effect=results) as run, \
                    contextlib.redirect_stdout(io.StringIO()) as stdout:
                self.assertEqual(main(), 0)
            self.assertEqual(run.call_count, 2)
            self.assertEqual(run.call_args_list[0].args[0],
                             ["git", "-C", str(upstream.resolve()), "rev-parse", "HEAD"])
            self.assertEqual(json.loads(run.call_args.kwargs["input"]), self.fixture)
            self.assertEqual(run.call_args.kwargs["encoding"], "utf-8")
            self.assertIn(f"{len(self.fixture['cases'])} saved selector expressions and 4 syntax failures", stdout.getvalue())
            self.assertIn("4 separate upstream index-limit errors", stdout.getvalue())

    def test_wrong_checkout_revision_fails_before_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            upstream = Path(directory)
            module = upstream / "yt_dlp/YoutubeDL.py"
            module.parent.mkdir(parents=True)
            module.touch()
            with mock.patch("sys.argv", ["compare_selectors.py", "--upstream", directory]), \
                    mock.patch("compare_selectors.subprocess.run", return_value=subprocess.CompletedProcess(
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
            with mock.patch("sys.argv", ["compare_selectors.py", "--upstream", directory]), \
                    mock.patch("compare_selectors.FIXTURE", fixture_path), \
                    mock.patch("compare_selectors.subprocess.run", return_value=subprocess.CompletedProcess(
                        [], 0, PINNED_UPSTREAM, "")) as run, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                self.assertEqual(main(), 1)
            self.assertEqual(run.call_count, 1)
            self.assertIn("fixture does not match", stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
