import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_archive import FIXTURE, PINNED_UPSTREAM, compare, main


class ArchiveComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
        self.result = {
            "identities": [{"name": case["name"], "archive_id": case["expected_archive_id"],
                            "file_hit": case["expected_hit"], "set_hit": case["expected_hit"]}
                           for case in self.fixture["cases"]],
            "records": [{"name": case["name"], "archive_id": case["expected_archive_id"],
                         "last_record": case["expected_archive_id"], "file_hit": True, "set_hit": True}
                        for case in self.fixture["record_cases"]],
            "flows": self.fixture["reference_flows"],
        }

    def test_exact_id_case_membership_and_record_results_without_mutating_input(self):
        before = copy.deepcopy((self.fixture, self.result))
        self.assertEqual(compare(self.result, self.fixture), self.result)
        self.assertEqual((self.fixture, self.result), before)
        changes = []
        for field, value in (("archive_id", "youtube caseid"), ("file_hit", False), ("set_hit", False)):
            changed = copy.deepcopy(self.result)
            changed["identities"][0][field] = value
            changes.append(changed)
        changed = copy.deepcopy(self.result)
        changed["records"][0]["last_record"] = "wrong recorded ID"
        changes.append(changed)
        changed = copy.deepcopy(self.result)
        changed["flows"][1]["archive_changed"] = True
        changes.append(changed)
        for result in changes:
            with self.subTest(result=result), self.assertRaisesRegex(ValueError, "identity/membership/control-flow mismatch"):
                compare(result, self.fixture)

    def test_shared_cases_preserve_id_case_unicode_and_internal_spaces(self):
        cases = {case["name"]: case for case in self.fixture["cases"]}
        self.assertEqual(cases["plain"]["expected_archive_id"], "youtube CaseID")
        self.assertEqual(cases["extractor-case"]["expected_archive_id"], "youtube CaseID")
        self.assertEqual(cases["id-case"]["expected_archive_id"], "youtube caseid")
        self.assertTrue(cases["plain"]["expected_hit"])
        self.assertFalse(cases["id-case"]["expected_hit"])
        self.assertEqual(cases["unicode"]["expected_archive_id"], "üniçode Café 影像")
        self.assertEqual(cases["greek-key-case"]["expected_archive_id"], "ος Greek")
        self.assertEqual(cases["internal-spaces"]["expected_archive_id"], "custom media Episode A")
        self.assertEqual(cases["double-internal-spaces"]["expected_archive_id"], "media  two inner  spaces")
        self.assertEqual(cases["numeric-id-preserved"]["expected_archive_id"], "youtube 001")
        for case in self.fixture["cases"]:
            info = case["info"]
            self.assertEqual(info["id"], info["id"].strip())
            self.assertEqual(info["extractor_key"], info["extractor_key"].strip())
            self.assertNotIn("ie_key", info)
            self.assertNotIn("_old_archive_ids", info)

    def test_archive_fixture_contains_mixed_newlines_whitespace_and_unrelated_records(self):
        contents = self.fixture["archive_contents"]
        self.assertIn("\r\n", contents)
        self.assertIn("Episode A\rüniçode", contents)
        self.assertIn(" \t\r\n", contents)
        self.assertIn("   youtube CaseID \t", contents)
        self.assertIn("\x1cyoutube 001\x1f", contents)
        self.assertIn("malformed-unrelated-record", contents)
        self.assertTrue(contents.endswith("\n"))  # Missing-newline repair is independent native policy.
        self.assertEqual(self.fixture["upstream_commit"], PINNED_UPSTREAM)

    def test_reference_flow_reads_skip_before_selection_and_do_not_implicitly_record(self):
        flows = {flow["name"]: flow for flow in self.fixture["reference_flows"]}
        hit = flows["archived-before-selection"]
        self.assertEqual((hit["selection_calls"], hit["listing_calls"], hit["record_calls"]), (0, 0, 0))
        self.assertTrue(hit["result_hit"])
        for name in ("plain-simulation-no-write", "listing-no-write"):
            self.assertFalse(flows[name]["archive_changed"])
            self.assertEqual(flows[name]["record_calls"], 0)
        self.assertEqual(flows["plain-simulation-no-write"]["selection_calls"], 1)
        self.assertEqual(flows["listing-no-write"]["listing_calls"], 1)
        forced = flows["reference-forced-simulation-write"]
        self.assertTrue(forced["archive_changed"])
        self.assertEqual(forced["record_calls"], 1)

    def test_command_uses_exact_fixture_and_pinned_actual_reference_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            upstream = Path(directory)
            module = upstream / "yt_dlp/YoutubeDL.py"
            module.parent.mkdir(parents=True)
            module.touch()
            results = [subprocess.CompletedProcess([], 0, PINNED_UPSTREAM + "\n", ""),
                       subprocess.CompletedProcess([], 0, json.dumps(self.result), "")]
            with mock.patch("sys.argv", ["compare_archive.py", "--upstream", directory]), \
                    mock.patch("compare_archive.subprocess.run", side_effect=results) as run, \
                    contextlib.redirect_stdout(io.StringIO()) as stdout:
                self.assertEqual(main(), 0)
            self.assertEqual(run.call_count, 2)
            self.assertEqual(run.call_args_list[0].args[0],
                             ["git", "-C", str(upstream.resolve()), "rev-parse", "HEAD"])
            self.assertEqual(json.loads(run.call_args.kwargs["input"]), self.fixture)
            self.assertEqual(run.call_args.kwargs["encoding"], "utf-8")
            self.assertIn("12 identities/hits", stdout.getvalue())

    def test_wrong_checkout_revision_fails_before_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            upstream = Path(directory)
            module = upstream / "yt_dlp/YoutubeDL.py"
            module.parent.mkdir(parents=True)
            module.touch()
            with mock.patch("sys.argv", ["compare_archive.py", "--upstream", directory]), \
                    mock.patch("compare_archive.subprocess.run", return_value=subprocess.CompletedProcess(
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
            fixture_path = upstream / "bad-fixture.json"
            self.fixture["upstream_commit"] = "0" * 40
            fixture_path.write_text(json.dumps(self.fixture), encoding="utf-8")
            with mock.patch("sys.argv", ["compare_archive.py", "--upstream", directory]), \
                    mock.patch("compare_archive.FIXTURE", fixture_path), \
                    mock.patch("compare_archive.subprocess.run", return_value=subprocess.CompletedProcess(
                        [], 0, PINNED_UPSTREAM, "")) as run, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                self.assertEqual(main(), 1)
            self.assertEqual(run.call_count, 1)
            self.assertIn("fixture does not match", stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
