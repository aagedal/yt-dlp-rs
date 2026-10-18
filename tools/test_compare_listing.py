import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_listing import FIELDS, FIXTURES, FLOW_CASES, PINNED_UPSTREAM, compare, main, normalize_formats


class ListingComparisonTests(unittest.TestCase):
    def setUp(self):
        self.info = json.loads((FIXTURES / "input.json").read_text(encoding="utf-8"))
        self.expected = json.loads((FIXTURES / "expected.json").read_text(encoding="utf-8"))
        self.result = {"direct": {"declaration_ids": self.expected["declaration_ids"],
                                  "formats": self.info["formats"]}, "flows": self.expected["flows"]}

    def test_metadata_and_declaration_order_match_without_mutating_inputs(self):
        before = copy.deepcopy((self.result, self.expected))
        actual = compare(self.result, self.expected)
        self.assertEqual(actual["formats"], self.expected["formats"])
        self.assertEqual((self.result, self.expected), before)
        self.assertEqual(actual["declaration_ids"], ["a64", "a128", "v360", "v720"])
        native = copy.deepcopy(self.info["formats"])
        for fmt in native:
            fmt.pop("acodec")
            fmt.pop("vcodec")
        self.assertEqual(normalize_formats(native), self.expected["formats"])

    def test_metadata_selection_events_and_order_changes_are_significant(self):
        changes = []
        for field, value in (("width", 641), ("filesize", 7), ("tbr", 1001), ("language", "de")):
            changed = copy.deepcopy(self.result)
            changed["direct"]["formats"][2][field] = value
            changes.append(changed)
        changed = copy.deepcopy(self.result)
        changed["direct"]["declaration_ids"].reverse()
        changes.append(changed)
        changed = copy.deepcopy(self.result)
        changed["flows"][0]["selection_calls"] = 1
        changes.append(changed)
        changed = copy.deepcopy(self.result)
        changed["flows"][3]["events"] = ["select", "list", "table", "json"]
        changes.append(changed)
        for result in changes:
            with self.subTest(result=result), self.assertRaisesRegex(ValueError, "metadata/control-flow mismatch"):
                compare(result, self.expected)

    def test_source_flags_distinguish_list_only_selected_and_print_json_paths(self):
        flows = {flow["name"]: flow for flow in self.expected["flows"]}
        ids = sorted(self.expected["declaration_ids"])
        for name in ("plain", "valid-list-only", "unavailable-list-only"):
            self.assertEqual(flows[name]["simulate"], "list_only")
            self.assertEqual(flows[name]["selection_calls"], 0)
            self.assertEqual(flows[name]["listed_ids"], ids)
            self.assertEqual(flows[name]["selected_ids"], [])
            self.assertEqual(flows[name]["events"], ["list", "table"])
        for name in ("json-pair", "simulate-pair"):
            self.assertIs(flows[name]["simulate"], True)
            self.assertEqual(flows[name]["selection_calls"], 1)
            self.assertEqual(flows[name]["selected_ids"], ["v720", "a128"])
            self.assertEqual(flows[name]["events"][:3], ["list", "table", "select"])
        self.assertEqual(flows["json-pair"]["json_selected_ids"], ["v720", "a128"])
        self.assertIsNone(flows["simulate-pair"]["json_selected_ids"])
        self.assertEqual(flows["reference-print-json"]["json_selected_ids"], [])
        self.assertEqual(flows["reference-print-json"]["selection_calls"], 0)
        self.assertEqual(flows["reference-print-json"]["events"], ["list", "table", "json"])
        self.assertEqual(flows["combined-unavailable"]["status"], "unavailable")
        self.assertEqual(flows["combined-unavailable"]["events"], ["list", "table", "select"])
        self.assertEqual(flows["malformed-before-listing"]["status"], "syntax_error")
        self.assertEqual(flows["malformed-before-listing"]["events"], [])
        self.assertEqual({name for name, _ in FLOW_CASES}, set(flows))

    def test_reference_only_fields_do_not_claim_rendered_cell_or_sorter_parity(self):
        formats = copy.deepcopy(self.info["formats"])
        for fmt in formats:
            fmt.update(protocol="https", resolution="reference-specific", filesize_approx=999,
                       abr=321, vbr=543, fps=29.97)
        self.assertEqual(normalize_formats(formats), self.expected["formats"])
        self.assertNotIn("abr", FIELDS)
        self.assertNotIn("resolution", FIELDS)
        self.assertEqual(self.expected["upstream_commit"], PINNED_UPSTREAM)
        invalid = [{"format_id": "unclassified"}]
        with self.assertRaisesRegex(ValueError, "stream classification"):
            normalize_formats(invalid)

    def test_command_uses_actual_worker_and_exact_shared_input_at_pinned_revision(self):
        with tempfile.TemporaryDirectory() as directory:
            upstream = Path(directory)
            module = upstream / "yt_dlp/YoutubeDL.py"
            module.parent.mkdir(parents=True)
            module.touch()
            results = [subprocess.CompletedProcess([], 0, PINNED_UPSTREAM + "\n", ""),
                       subprocess.CompletedProcess([], 0, json.dumps(self.result), "")]
            with mock.patch("sys.argv", ["compare_listing.py", "--upstream", directory]), \
                    mock.patch("compare_listing.subprocess.run", side_effect=results) as run, \
                    contextlib.redirect_stdout(io.StringIO()) as stdout:
                self.assertEqual(main(), 0)
            self.assertEqual(run.call_count, 2)
            self.assertEqual(run.call_args_list[0].args[0],
                             ["git", "-C", str(upstream.resolve()), "rev-parse", "HEAD"])
            self.assertEqual(json.loads(run.call_args.kwargs["input"])["info"], self.info)
            self.assertEqual(run.call_args.kwargs["encoding"], "utf-8")
            self.assertIn("8 pinned upstream control flows", stdout.getvalue())

    def test_bad_checkout_revision_fails_before_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            upstream = Path(directory)
            module = upstream / "yt_dlp/YoutubeDL.py"
            module.parent.mkdir(parents=True)
            module.touch()
            with mock.patch("sys.argv", ["compare_listing.py", "--upstream", directory]), \
                    mock.patch("compare_listing.subprocess.run", return_value=subprocess.CompletedProcess(
                        [], 0, "0" * 40, "")) as run, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                self.assertEqual(main(), 1)
            self.assertEqual(run.call_count, 1)
            self.assertIn(PINNED_UPSTREAM, stderr.getvalue())

    def test_bad_fixture_revision_fails_before_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            upstream = Path(directory)
            module = upstream / "yt_dlp/YoutubeDL.py"
            module.parent.mkdir(parents=True)
            module.touch()
            fixtures = upstream / "fixtures"
            fixtures.mkdir()
            (fixtures / "input.json").write_text(json.dumps(self.info), encoding="utf-8")
            self.expected["upstream_commit"] = "0" * 40
            (fixtures / "expected.json").write_text(json.dumps(self.expected), encoding="utf-8")
            with mock.patch("sys.argv", ["compare_listing.py", "--upstream", directory]), \
                    mock.patch("compare_listing.FIXTURES", fixtures), \
                    mock.patch("compare_listing.subprocess.run", return_value=subprocess.CompletedProcess(
                        [], 0, PINNED_UPSTREAM, "")) as run, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                self.assertEqual(main(), 1)
            self.assertEqual(run.call_count, 1)
            self.assertIn("fixture does not match", stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
