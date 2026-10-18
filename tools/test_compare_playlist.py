import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_playlist import FIXTURE, PINNED_UPSTREAM, compare, main


class PlaylistComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
        def results(key):
            return [{"name": case["name"], "indices": case["expected_indices"],
                     "warning": case["expected_warning"]} for case in self.fixture[key]]
        self.result = {"matches": results("cases"), "source_only_matches": results("source_only_cases"),
                       "invalid": [case["name"] for case in self.fixture["invalid_cases"]]}

    def test_exact_phase_order_dedup_and_warning_results_are_compared_without_mutation(self):
        before = copy.deepcopy((self.fixture, self.result))
        self.assertEqual(compare(self.result, self.fixture), self.result)
        self.assertEqual((self.fixture, self.result), before)
        changes = []
        changed = copy.deepcopy(self.result)
        changed["matches"][0]["indices"].reverse()
        changes.append(changed)
        changed = copy.deepcopy(self.result)
        changed["matches"][0]["warning"] = True
        changes.append(changed)
        changed = copy.deepcopy(self.result)
        changed["invalid"].pop()
        changes.append(changed)
        changed = copy.deepcopy(self.result)
        changed["source_only_matches"][0]["indices"] = []
        changes.append(changed)
        for result in changes:
            with self.subTest(result=result), self.assertRaisesRegex(ValueError, "playlist selection mismatch"):
                compare(result, self.fixture)

    def test_signed_ranges_keep_clipping_phase_and_request_order_before_reverse(self):
        cases = {case["name"]: case for case in self.fixture["cases"]}
        for name, indices in {
            'inclusive-colon': [1, 2, 3], 'inclusive-hyphen': [1, 2, 3],
            'negative-phase-clip': [1, 3, 5, 7, 9],
            'reverse-phase-clip': [8, 6, 4, 2, 0],
            'negative-phase-jump': [7], 'reverse-phase-jump': [2],
            'zero-phase': [1], 'zero-nohit': [], 'negative-zero-nohit': [],
            'duplicates-request-order': [3, 1, 0, 2],
            'reverse-duplicate-order': [2, 0, 1, 3],
            'nohits-all': [], 'signed-plus-range': [0, 2],
        }.items():
            self.assertEqual(cases[name]["expected_indices"], indices, name)
        for case in self.fixture["cases"]:
            indices = case["expected_indices"]
            self.assertEqual(len(indices), len(set(indices)), case["name"])
            self.assertTrue(all(0 <= index < case["length"] for index in indices), case["name"])
            if case["length"] == 0:
                self.assertEqual(indices, [])

    def test_inf_infinite_reverse_quirk_is_preserved_instead_of_normalized(self):
        cases = {case["name"]: case for case in self.fixture["cases"]}
        self.assertEqual(cases['positive-inf']["expected_indices"], [0, 2, 4, 6, 8])
        self.assertEqual(cases['positive-infinite']["expected_indices"], [0, 2, 4, 6, 8])
        self.assertEqual(cases['negative-step-inf-empty']["expected_indices"], [])
        self.assertEqual(cases['negative-step-infinite-open']["expected_indices"], list(range(9, -1, -1)))
        self.assertEqual(cases['all-hyphen']["expected_indices"], list(range(10)))
        self.assertEqual(cases['legacy-missing-start-step']["expected_indices"], [0, 2, 4, 6, 8])

    def test_cli_validation_is_stricter_than_falsy_low_level_items_and_checks_legacy_bounds(self):
        valid = {case["name"]: case for case in self.fixture["cases"]}
        invalid = {case["name"]: case for case in self.fixture["invalid_cases"]}
        self.assertIn('', [case["items"] for case in invalid.values()])
        self.assertIn(' ', [case["items"] for case in invalid.values()])
        self.assertIn('1, 2', [case["items"] for case in invalid.values()])
        self.assertEqual(invalid['invalid-legacy-start-zero']["start"], 0)
        self.assertEqual(valid['zero-nohit']["items"], '0')
        self.assertEqual(valid['zero-nohit']["start"], 1)
        override = invalid['invalid-bounds-with-override']
        self.assertEqual(override["items"], '1')
        self.assertGreater(override["start"], override["end"])
        self.assertTrue(valid['items-override-start']["expected_warning"])
        self.assertTrue(valid['items-override-end']["expected_warning"])
        self.assertFalse(valid['legacy-unbounded-end']["expected_warning"])
        for case in self.fixture["source_only_cases"]:
            self.assertEqual(case["expected_indices"], [0, 1, 2])
            self.assertIn('Non-ASCII', case["native_policy"])
        self.assertEqual(self.fixture["upstream_commit"], PINNED_UPSTREAM)

    def test_command_uses_actual_worker_and_exact_shared_fixture(self):
        with tempfile.TemporaryDirectory() as directory:
            upstream = Path(directory)
            module = upstream / "yt_dlp/utils/_utils.py"
            module.parent.mkdir(parents=True)
            module.touch()
            results = [subprocess.CompletedProcess([], 0, PINNED_UPSTREAM + "\n", ""),
                       subprocess.CompletedProcess([], 0, json.dumps(self.result), "")]
            with mock.patch("sys.argv", ["compare_playlist.py", "--upstream", directory]), \
                    mock.patch("compare_playlist.subprocess.run", side_effect=results) as run, \
                    contextlib.redirect_stdout(io.StringIO()) as stdout:
                self.assertEqual(main(), 0)
            self.assertEqual(run.call_count, 2)
            self.assertEqual(run.call_args_list[0].args[0],
                             ["git", "-C", str(upstream.resolve()), "rev-parse", "HEAD"])
            self.assertEqual(json.loads(run.call_args.kwargs["input"]), self.fixture)
            self.assertEqual(run.call_args.kwargs["encoding"], "utf-8")
            self.assertIn('79 selections and 24 invalid CLI cases', stdout.getvalue())
            self.assertIn('2 source-only Unicode cases', stdout.getvalue())

    def test_wrong_checkout_revision_fails_before_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            upstream = Path(directory)
            module = upstream / "yt_dlp/utils/_utils.py"
            module.parent.mkdir(parents=True)
            module.touch()
            with mock.patch("sys.argv", ["compare_playlist.py", "--upstream", directory]), \
                    mock.patch("compare_playlist.subprocess.run", return_value=subprocess.CompletedProcess(
                        [], 0, "0" * 40, "")) as run, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                self.assertEqual(main(), 1)
            self.assertEqual(run.call_count, 1)
            self.assertIn(PINNED_UPSTREAM, stderr.getvalue())

    def test_wrong_fixture_revision_fails_before_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            upstream = Path(directory)
            module = upstream / "yt_dlp/utils/_utils.py"
            module.parent.mkdir(parents=True)
            module.touch()
            fixture_path = upstream / 'wrong-fixture.json'
            self.fixture['upstream_commit'] = '0' * 40
            fixture_path.write_text(json.dumps(self.fixture), encoding='utf-8')
            with mock.patch("sys.argv", ["compare_playlist.py", "--upstream", directory]), \
                    mock.patch('compare_playlist.FIXTURE', fixture_path), \
                    mock.patch("compare_playlist.subprocess.run", return_value=subprocess.CompletedProcess(
                        [], 0, PINNED_UPSTREAM, "")) as run, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                self.assertEqual(main(), 1)
            self.assertEqual(run.call_count, 1)
            self.assertIn('fixture does not match', stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
