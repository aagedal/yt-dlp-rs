import copy
import json
from pathlib import Path
import subprocess
import sys
import unittest
from unittest import mock

from compare_ejs import (
    DENO_ARGS, ROOT, UPSTREAM_WORKER, compare, parse_output, run_deno, runtime_environment,
    verify_inputs,
)


class EjsComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture = verify_inputs(ROOT)
        self.output = {
            "type": "result",
            "responses": [
                {"type": "result", "data": dict(request["challenges"])}
                for request in self.fixture["requests"]
            ],
        }

    def test_only_preprocessed_source_is_ignored_without_mutation(self):
        original = copy.deepcopy(self.output)
        self.output["preprocessed_player"] = "arbitrary prepared player source"
        before = copy.deepcopy(self.output)
        self.assertEqual(compare(self.output, self.fixture), original)
        self.assertEqual(self.output, before)
        self.output["extra"] = "unexpected"
        with self.assertRaises(ValueError):
            compare(self.output, self.fixture)

    def test_exact_results_keys_and_response_order_are_significant(self):
        cases = []
        cases.append(dict(self.output, responses=self.output["responses"][::-1]))
        cases.append(dict(self.output, responses=self.output["responses"][:1]))
        for replacement in ({}, {"wrong": "key"}, {"extra": "value"}):
            changed = copy.deepcopy(self.output)
            changed["responses"][0]["data"] = replacement
            cases.append(changed)
        for value in (None, 7, "different", ""):
            changed = copy.deepcopy(self.output)
            first = next(iter(changed["responses"][0]["data"]))
            changed["responses"][0]["data"][first] = value
            cases.append(changed)
        cases.append(dict(self.output, preprocessed_player=None))
        cases.append({"type": "error", "error": "failed"})
        for changed in cases:
            with self.subTest(output=changed), self.assertRaises(ValueError):
                compare(changed, self.fixture)

    def test_json_rejects_duplicate_escaped_aliases_and_trailing_output(self):
        for invalid in (
            '{"type":"result","type":"error"}',
            '{"data":{"a":"one","\\u0061":"two"}}',
            '{} {}',
            b'\xff',
        ):
            with self.subTest(output=invalid), self.assertRaises(ValueError):
                parse_output(invalid)
        self.assertEqual(compare(parse_output(json.dumps(self.output)), self.fixture), self.output)

    def test_runtime_environment_clears_ambient_flags_and_proxy(self):
        with mock.patch.dict("os.environ", {"DENO_FLAGS": "--allow-all", "HTTPS_PROXY": "proxy"}):
            environment = runtime_environment("private-directory")
        self.assertEqual(environment["DENO_DIR"], "private-directory")
        self.assertEqual(environment["DENO_NO_UPDATE_CHECK"], "1")
        self.assertNotIn("DENO_FLAGS", environment)
        self.assertNotIn("HTTPS_PROXY", environment)
        self.assertNotIn("PATH", environment)

    def test_worker_socket_guard_allows_ssl_import_but_denies_network(self):
        # Replacing socket.socket with a function breaks ssl's subclass at import.
        prelude = UPSTREAM_WORKER.split("sys.path.insert", 1)[0]
        script = prelude + '''
import ssl
for operation in (socket.socket, lambda: socket.create_connection(('localhost', 80))):
    try:
        operation()
    except AssertionError:
        pass
    else:
        raise AssertionError('network guard did not reject operation')
'''
        subprocess.run([sys.executable, "-B", "-c", script],
                       capture_output=True, text=True, check=True, timeout=15)

    def test_deno_uses_pinned_version_offline_flags_and_cleans_directory(self):
        completed = [
            subprocess.CompletedProcess([], 0, "deno 2.9.7 (stable, release, x86_64-pc-windows-msvc)\n", ""),
            subprocess.CompletedProcess([], 0, json.dumps(self.output), ""),
        ]
        directories = []

        def run(*args, **kwargs):
            self.assertTrue(Path(kwargs["cwd"]).is_dir())
            self.assertEqual(kwargs["env"]["DENO_DIR"], kwargs["cwd"])
            self.assertEqual(kwargs["timeout"], 15)
            directories.append(Path(kwargs["cwd"]))
            return completed.pop(0)

        with mock.patch("compare_ejs.subprocess.run", side_effect=run) as mocked:
            self.assertEqual(run_deno(Path("explicit-deno"), "script source"), self.output)
        self.assertEqual(mocked.call_args_list[0].args[0], ["explicit-deno", "--version"])
        self.assertEqual(mocked.call_args_list[1].args[0], ["explicit-deno", *DENO_ARGS])
        self.assertEqual(mocked.call_args_list[1].kwargs["input"], "script source")
        self.assertTrue(all(not directory.exists() for directory in directories))

    def test_bad_version_stderr_and_process_failure_are_errors(self):
        for version in ("deno 2.9.6 (stable, release)\n", "", "node 2.9.7\n"):
            with mock.patch("compare_ejs.subprocess.run", return_value=
                            subprocess.CompletedProcess([], 0, version, "")) as run:
                with self.assertRaisesRegex(ValueError, "pinned Deno"):
                    run_deno(Path("deno"), "script")
                self.assertEqual(run.call_count, 1)
        good_version = subprocess.CompletedProcess([], 0, "deno 2.9.7 (stable, release)\n", "")
        with mock.patch("compare_ejs.subprocess.run", side_effect=[
            good_version, subprocess.CompletedProcess([], 0, json.dumps(self.output), "warning"),
        ]):
            with self.assertRaisesRegex(ValueError, "stderr"):
                run_deno(Path("deno"), "script")
        with mock.patch("compare_ejs.subprocess.run", side_effect=
                        subprocess.TimeoutExpired("deno", 15)):
            with self.assertRaises(subprocess.TimeoutExpired):
                run_deno(Path("deno"), "script")


if __name__ == "__main__":
    unittest.main()
