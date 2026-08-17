"""Red-phase integration tests for E33-H02's assertable runner.

These tests deliberately exercise the real ``lodestar_harness.py`` as a
subprocess.  The corpus and batch are ephemeral so the test does not depend on
the private homelab or on a checkout-specific root.
"""

from __future__ import annotations

import json
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock


REPO_ROOT = Path(__file__).resolve().parents[1]
HARNESS = REPO_ROOT / "docs" / "qa" / "testbench" / "lodestar_harness.py"
BATCHES = HARNESS.parent / "batches"
BUILD_ARTIFACT = HARNESS.parent / "build_artifact.py"


def load_harness_module(suffix: str):
    module_spec = importlib.util.spec_from_file_location(
        f"lodestar_harness_{suffix}", HARNESS
    )
    if module_spec is None or module_spec.loader is None:
        raise AssertionError(f"could not load harness module from {HARNESS}")
    harness = importlib.util.module_from_spec(module_spec)
    with mock.patch.object(sys, "path", [str(HARNESS.parent), *sys.path]):
        module_spec.loader.exec_module(harness)
    return harness


def run_harness(*arguments: str, env: dict[str, str] | None = None):
    return subprocess.run(
        [sys.executable, str(HARNESS), *arguments],
        cwd=REPO_ROOT,
        env={**os.environ, **(env or {})},
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )


class AssertableRunnerTest(unittest.TestCase):
    maxDiff = None

    def setUp(self) -> None:
        binary_from_env = os.environ.get("LODESTAR_MCP_BIN")
        self.assertIsNotNone(
            binary_from_env,
            "anti-vacuity: set LODESTAR_MCP_BIN to the real lodestar-mcp binary",
        )
        self.binary = Path(binary_from_env).resolve()
        self.assertTrue(HARNESS.is_file(), f"anti-vacuity: harness missing: {HARNESS}")
        self.assertTrue(self.binary.is_file(), f"anti-vacuity: binary missing: {self.binary}")
        self.assertTrue(
            os.access(self.binary, os.X_OK),
            f"anti-vacuity: binary is not executable: {self.binary}",
        )

    def _run_meta_case(self, case_id: str, expected_documents: int):
        with tempfile.TemporaryDirectory(prefix="lodestar-e33-h02-") as temp:
            temp_path = Path(temp)
            corpus = temp_path / "corpus"
            corpus.mkdir()
            document = corpus / "documento.md"
            document.write_text("# Documento de autotest\n", encoding="utf-8")
            original_bytes = document.read_bytes()

            batch = temp_path / "meta_runner.json"
            results = temp_path / "results.json"
            batch.write_text(
                json.dumps(
                    {
                        "batch": "meta_runner",
                        "root": "real",
                        "profile": "readonly",
                        "cases": [
                            {
                                "id": case_id,
                                "tool": "workspace_status",
                                "arguments": {},
                                "expect": {
                                    "is_error": False,
                                    "equals": {
                                        "structured.counts.documents": expected_documents,
                                    },
                                    "present": ["structured.workspaceRevision"],
                                },
                            }
                        ],
                    },
                    indent=2,
                )
                + "\n",
                encoding="utf-8",
            )

            module_spec = importlib.util.spec_from_file_location(
                f"lodestar_harness_{case_id.lower().replace('-', '_')}", HARNESS
            )
            self.assertIsNotNone(module_spec)
            self.assertIsNotNone(module_spec.loader)
            harness = importlib.util.module_from_spec(module_spec)
            # Loading a script through importlib does not add its directory to
            # sys.path as normal script execution does.  H02 splits the evaluator
            # into runner_expect.py next to the harness, so reproduce that import
            # environment explicitly.
            with mock.patch.object(sys, "path", [str(HARNESS.parent), *sys.path]):
                module_spec.loader.exec_module(harness)

            # Patch the legacy globals only to let the pre-H02 harness reach
            # expect evaluation.  The public inputs exercised are still the
            # ratified --root argument and LODESTAR_MCP_BIN environment value;
            # once H02 exists those are what main() must use.
            harness.BINARY = str(self.binary)
            harness.HOMELAB = str(corpus)
            stdout = io.StringIO()
            stderr = io.StringIO()
            returncode = 0
            argv = [
                str(HARNESS),
                "--root",
                str(corpus),
                "--batch",
                str(batch),
                "--out",
                str(results),
            ]
            with (
                mock.patch.object(sys, "argv", argv),
                mock.patch.dict(os.environ, {"LODESTAR_MCP_BIN": str(self.binary)}),
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                try:
                    outcome = harness.main()
                    if isinstance(outcome, int):
                        returncode = outcome
                except SystemExit as error:
                    returncode = error.code if isinstance(error.code, int) else 1
            completed = SimpleNamespace(
                returncode=returncode,
                stdout=stdout.getvalue(),
                stderr=stderr.getvalue(),
            )

            self.assertTrue(
                results.is_file(),
                "runner did not emit its machine-readable report; the portable "
                f"--binary/--root invocation failed before verdict evaluation\n"
                f"exit={completed.returncode}\nstdout={completed.stdout}\n"
                f"stderr={completed.stderr}",
            )
            try:
                report = json.loads(results.read_text(encoding="utf-8"))
            except json.JSONDecodeError as error:
                self.fail(f"runner report is not JSON: {error}\n{results.read_text(encoding='utf-8')}")

            rendered = json.dumps(report, ensure_ascii=False, sort_keys=True)
            self.assertNotIn("session_error", rendered, "anti-vacuity: MCP session never started")
            self.assertNotIn(
                "harness_exception", rendered, "anti-vacuity: tool call did not complete"
            )
            self.assertEqual([case_id], [case.get("id") for case in report.get("cases", [])])

            steps = report["cases"][0].get("steps", [])
            self.assertEqual(1, len(steps), "anti-vacuity: workspace_status was not called once")
            structured = steps[0].get("structured")
            self.assertIsInstance(structured, dict, "anti-vacuity: real MCP response is absent")
            self.assertEqual(
                os.path.realpath(corpus),
                os.path.realpath(structured.get("root", "")),
                "runner used a root other than the ephemeral corpus passed with --root",
            )
            self.assertEqual(
                original_bytes,
                document.read_bytes(),
                "readonly runner modified the declared real root",
            )
            return completed, report

    def test_bdd_1_meta_01_matching_expect_is_pass_and_exit_zero(self) -> None:
        completed, report = self._run_meta_case("META-01", expected_documents=1)

        self.assertEqual(
            "PASS",
            report["cases"][0].get("verdict"),
            f"matching expect did not produce PASS: {json.dumps(report, indent=2)}",
        )
        self.assertEqual(
            0,
            completed.returncode,
            f"matching expect must exit 0\nstdout={completed.stdout}\nstderr={completed.stderr}",
        )

    def test_bdd_2_meta_02_false_expect_is_fail_names_subfield_and_exits_nonzero(self) -> None:
        completed, report = self._run_meta_case("META-02", expected_documents=999)

        self.assertEqual(
            "FAIL",
            report["cases"][0].get("verdict"),
            f"false expect did not produce FAIL: {json.dumps(report, indent=2)}",
        )
        evidence = "\n".join(
            (json.dumps(report, ensure_ascii=False), completed.stdout, completed.stderr)
        )
        self.assertIn(
            "structured.counts.documents",
            evidence,
            "FAIL detail does not name the discrepant structured subfield",
        )
        self.assertNotEqual(
            0,
            completed.returncode,
            "a batch containing an evaluated FAIL must exit non-zero",
        )

    def test_structural_binary_and_real_root_are_portable_cli_arguments(self) -> None:
        with tempfile.TemporaryDirectory(prefix="lodestar-e33-h02-cli-") as temp:
            corpus = Path(temp) / "corpus"
            corpus.mkdir()
            (corpus / "documento.md").write_text("# Documento\n", encoding="utf-8")

            completed = subprocess.run(
                [
                    sys.executable,
                    str(HARNESS),
                    "--binary",
                    str(self.binary),
                    "--root",
                    str(corpus),
                    "--list-tools",
                ],
                cwd=REPO_ROOT,
                capture_output=True,
                text=True,
                timeout=30,
                check=False,
            )

        self.assertEqual(
            0,
            completed.returncode,
            "portable CLI rejected --binary/--root or failed to start the real binary\n"
            f"stdout={completed.stdout}\nstderr={completed.stderr}",
        )
        listed = json.loads(completed.stdout)
        self.assertIn("workspace_status", listed.get("tools", []))


class RunnerRepairRegressionTest(unittest.TestCase):
    maxDiff = None

    def test_exploratory_batch_with_executable_that_is_not_mcp_is_bank_execution_error(self):
        with tempfile.TemporaryDirectory(prefix="lodestar-e33-h02-non-mcp-") as temp:
            base = Path(temp)
            corpus = base / "corpus"
            corpus.mkdir()
            fake_mcp = base / "not-an-mcp"
            fake_mcp.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            fake_mcp.chmod(0o755)
            batch = base / "exploratory.json"
            batch.write_text(
                json.dumps(
                    {
                        "batch": "exploratory_non_mcp",
                        "root": "corpus",
                        "profile": "readonly",
                        "cases": [
                            {
                                "id": "EXPLORATORY-NON-MCP",
                                "tool": "workspace_status",
                                "arguments": {},
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )

            completed = run_harness(
                "--batch",
                str(batch),
                "--root-corpus",
                str(corpus),
                "--binary",
                str(fake_mcp),
            )

        self.assertEqual(
            3,
            completed.returncode,
            "a process that is executable but cannot speak MCP is an execution error of "
            "the bank, even when the case has no assertions\n"
            f"stdout={completed.stdout}\nstderr={completed.stderr}",
        )
        self.assertIn("ERROR DE EJECUCIÓN", completed.stderr)
        self.assertNotIn("EXPLOR ", completed.stdout)
        self.assertNotIn("PASS ", completed.stdout)

    def test_call_with_invalid_json_is_controlled_usage_error_without_traceback(self):
        with tempfile.TemporaryDirectory(prefix="lodestar-e33-h02-call-json-") as temp:
            base = Path(temp)
            root = base / "root"
            root.mkdir()
            binary_from_env = os.environ.get("LODESTAR_MCP_BIN")
            self.assertIsNotNone(
                binary_from_env,
                "anti-vacuity: set LODESTAR_MCP_BIN so invalid JSON reaches --call parsing",
            )
            real_mcp = Path(binary_from_env).resolve()
            self.assertTrue(real_mcp.is_file(), f"anti-vacuity: binary missing: {real_mcp}")
            self.assertTrue(os.access(real_mcp, os.X_OK))

            completed = run_harness(
                "--root",
                str(root),
                "--binary",
                str(real_mcp),
                "--call",
                "workspace_status",
                "{not-json",
            )

        self.assertEqual(2, completed.returncode, completed.stdout + completed.stderr)
        self.assertIn("USO:", completed.stderr)
        self.assertNotIn("Traceback", completed.stderr)

    def test_primary_modes_are_pairwise_mutually_exclusive_before_session_start(self):
        with tempfile.TemporaryDirectory(prefix="lodestar-e33-h02-exclusive-modes-") as temp:
            base = Path(temp)
            corpus = base / "corpus"
            corpus.mkdir()
            batch = base / "empty.json"
            batch.write_text(
                json.dumps(
                    {
                        "batch": "empty",
                        "root": "corpus",
                        "profile": "readonly",
                        "cases": [],
                    }
                ),
                encoding="utf-8",
            )
            fake_mcp = base / "fake-mcp"
            fake_mcp.write_text(
                "#!/bin/sh\n"
                ": > \"$E33_H02_MODE_SPAWN_MARKER\"\n"
                "exit 0\n",
                encoding="utf-8",
            )
            fake_mcp.chmod(0o755)
            modes = {
                "batch": ["--batch", str(batch)],
                "run-all": ["--run-all"],
                "call": ["--call", "workspace_status", "{}"],
                "list-tools": ["--list-tools"],
            }
            violations = []
            names = list(modes)
            for left_index, left in enumerate(names):
                for right in names[left_index + 1 :]:
                    marker = base / f"spawned-{left}-{right}"
                    completed = run_harness(
                        *modes[left],
                        *modes[right],
                        "--root",
                        str(corpus),
                        "--root-corpus",
                        str(corpus),
                        "--binary",
                        str(fake_mcp),
                        env={"E33_H02_MODE_SPAWN_MARKER": str(marker)},
                    )
                    label = f"--{left} + --{right}"
                    if completed.returncode != 2:
                        violations.append(
                            f"{label}: exit {completed.returncode}, expected usage exit 2"
                        )
                    if "USO:" not in completed.stderr:
                        violations.append(f"{label}: missing controlled USO diagnostic")
                    if "Traceback" in completed.stderr:
                        violations.append(f"{label}: leaked traceback")
                    if marker.exists():
                        violations.append(f"{label}: opened an MCP process before rejection")

        self.assertEqual(
            [],
            violations,
            "--batch/--run-all/--call/--list-tools are four mutually exclusive "
            "primary modes",
        )

    def test_preflight_rejects_internal_step_expect_types_before_execution(self):
        malformed_expectations = {
            "is_error bool": {"is_error": "false"},
            "error_code string": {"error_code": 7},
            "protocol_error_code int": {"protocol_error_code": "-32602"},
            "protocol_error_code rejects bool": {"protocol_error_code": False},
            "equals map": {"equals": ["structured.value", 1]},
            "present list": {"present": "structured.value"},
            "present path strings": {"present": [7]},
            "absent list": {"absent": {"structured.value": True}},
            "absent path strings": {"absent": [False]},
            "matches map": {"matches": ["stdout", "ok"]},
            "matches regex value string": {"matches": {"stdout": 7}},
            "matches rejects invalid regex": {"matches": {"stdout": "["}},
            "contains map": {"contains": ["stdout", "ok"]},
            "not_contains map": {"not_contains": ["stdout", "bad"]},
            "length map": {"length": ["stdout", 1]},
            "length inner int": {"length": {"stdout": True}},
            "min_length map": {"min_length": ["stdout", 1]},
            "min_length inner int": {"min_length": {"stdout": "1"}},
            "type map": {"type": ["stdout", "string"]},
            "type known name": {"type": {"stdout": "integer"}},
            "rc int": {"rc": False},
            "describe string": {"describe": ["not", "prose"]},
        }

        with tempfile.TemporaryDirectory(prefix="lodestar-e33-h02-expect-preflight-") as temp:
            base = Path(temp)
            corpus = base / "corpus"
            corpus.mkdir()
            violations = []
            for row_number, (label, malformed_expect) in enumerate(
                malformed_expectations.items()
            ):
                marker = base / f"STEP_RAN_{row_number}"
                batch = base / f"malformed-{row_number}.json"
                batch.write_text(
                    json.dumps(
                        {
                            "batch": f"malformed_{row_number}",
                            "root": "corpus",
                            "profile": "readonly",
                            "cases": [
                                {
                                    "id": f"MALFORMED-{row_number}",
                                    "no_server": True,
                                    "steps": [
                                        {
                                            "kind": "shell",
                                            "cmd": f"touch {shlex.quote(str(marker))}",
                                            "expect": malformed_expect,
                                        }
                                    ],
                                }
                            ],
                        }
                    ),
                    encoding="utf-8",
                )

                completed = run_harness(
                    "--batch",
                    str(batch),
                    "--root-corpus",
                    str(corpus),
                    "--binary",
                    sys.executable,
                )

                if completed.returncode != 2:
                    violations.append(f"{label}: exit {completed.returncode}, expected 2")
                if "USO:" not in completed.stderr:
                    violations.append(f"{label}: missing controlled USO diagnostic")
                if "Traceback" in completed.stderr:
                    violations.append(f"{label}: leaked traceback")
                if marker.exists():
                    violations.append(f"{label}: step executed before validation")

        self.assertEqual(
            [],
            violations,
            "every internal step.expect type must be rejected in preflight",
        )

    def test_preflight_rejects_malformed_invariant_internals_before_execution(self):
        malformed_invariants = {
            "invariant known string": {
                "invariant": "equal",
                "steps": [0, 1],
                "path": "stdout",
            },
            "steps list": {"invariant": "same", "steps": "0,1", "path": "stdout"},
            "steps minimum two": {"invariant": "same", "steps": [0], "path": "stdout"},
            "steps integer indices": {
                "invariant": "same",
                "steps": [0, "1"],
                "path": "stdout",
            },
            "steps reject boolean indices": {
                "invariant": "same",
                "steps": [0, True],
                "path": "stdout",
            },
            "steps existing indices": {
                "invariant": "same",
                "steps": [0, 2],
                "path": "stdout",
            },
            "path string": {"invariant": "same", "steps": [0, 1], "path": ["stdout"]},
            "describe string": {
                "invariant": "same",
                "steps": [0, 1],
                "path": "stdout",
                "describe": 9,
            },
        }

        with tempfile.TemporaryDirectory(prefix="lodestar-e33-h02-invariant-preflight-") as temp:
            base = Path(temp)
            corpus = base / "corpus"
            corpus.mkdir()
            violations = []
            for row_number, (label, malformed_invariant) in enumerate(
                malformed_invariants.items()
            ):
                marker = base / f"STEP_RAN_{row_number}"
                batch = base / f"malformed-invariant-{row_number}.json"
                batch.write_text(
                    json.dumps(
                        {
                            "batch": f"malformed_invariant_{row_number}",
                            "root": "corpus",
                            "profile": "readonly",
                            "cases": [
                                {
                                    "id": f"MALFORMED-INVARIANT-{row_number}",
                                    "no_server": True,
                                    "steps": [
                                        {
                                            "kind": "shell",
                                            "cmd": f"touch {shlex.quote(str(marker))}",
                                        },
                                        {"kind": "shell", "cmd": "true"},
                                    ],
                                    "expect": [malformed_invariant],
                                }
                            ],
                        }
                    ),
                    encoding="utf-8",
                )

                completed = run_harness(
                    "--batch",
                    str(batch),
                    "--root-corpus",
                    str(corpus),
                    "--binary",
                    sys.executable,
                )

                if completed.returncode != 2:
                    violations.append(f"{label}: exit {completed.returncode}, expected 2")
                if "USO:" not in completed.stderr:
                    violations.append(f"{label}: missing controlled USO diagnostic")
                if "Traceback" in completed.stderr:
                    violations.append(f"{label}: leaked traceback")
                if marker.exists():
                    violations.append(f"{label}: step executed before validation")

        self.assertEqual(
            [],
            violations,
            "every malformed invariant must be rejected before any step executes",
        )

    def test_public_binary_flag_precedence_over_environment_is_unambiguous(self):
        with tempfile.TemporaryDirectory(prefix="lodestar-e33-h02-binary-precedence-") as temp:
            base = Path(temp)
            corpus = base / "corpus"
            corpus.mkdir()
            valid_binary = base / "valid-executable"
            valid_binary.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            valid_binary.chmod(0o755)
            missing_binary = base / "missing-binary"
            batch = base / "empty.json"
            batch.write_text(
                json.dumps(
                    {
                        "batch": "binary_precedence",
                        "root": "corpus",
                        "profile": "readonly",
                        "cases": [],
                    }
                ),
                encoding="utf-8",
            )

            flag_wins = run_harness(
                "--batch",
                str(batch),
                "--root-corpus",
                str(corpus),
                "--binary",
                str(valid_binary),
                env={"LODESTAR_MCP_BIN": str(missing_binary)},
            )
            invalid_flag_wins = run_harness(
                "--batch",
                str(batch),
                "--root-corpus",
                str(corpus),
                "--binary",
                str(missing_binary),
                env={"LODESTAR_MCP_BIN": str(valid_binary)},
            )

        self.assertEqual(
            0,
            flag_wins.returncode,
            "a valid --binary must take precedence over an invalid environment value\n"
            f"stdout={flag_wins.stdout}\nstderr={flag_wins.stderr}",
        )
        self.assertEqual(
            3,
            invalid_flag_wins.returncode,
            "an invalid explicit --binary must not silently fall back to a valid environment "
            f"value\nstdout={invalid_flag_wins.stdout}\nstderr={invalid_flag_wins.stderr}",
        )

    def test_real_readonly_batch_rejects_mutating_shell_and_standard_spawn_before_steps(self):
        """A real root is readonly at the whole-batch boundary, including local steps."""
        with tempfile.TemporaryDirectory(prefix="lodestar-e33-h02-real-") as temp:
            base = Path(temp)
            root = base / "real-root"
            root.mkdir()
            shell_marker = root / "SHELL_STEP_RAN"
            spawn_marker = root / "SPAWN_STEP_RAN"
            fake_mcp = base / "fake-mcp"
            fake_mcp.write_text(
                "#!/bin/sh\n"
                "touch \"$E33_H02_SPAWN_MARKER\"\n"
                "exit 0\n",
                encoding="utf-8",
            )
            fake_mcp.chmod(0o755)
            batch = base / "real-readonly-mutators.json"
            batch.write_text(
                json.dumps(
                    {
                        "batch": "real_readonly_mutators",
                        "root": "real",
                        "profile": "readonly",
                        "cases": [
                            {
                                "id": "REAL-READONLY-MUTATORS",
                                "no_server": True,
                                "steps": [
                                    {
                                        "kind": "shell",
                                        "cmd": f"touch {shlex.quote(str(shell_marker))}",
                                        "expect": {"rc": 0},
                                    },
                                    {
                                        "kind": "spawn",
                                        "args": ["--root", "@root", "--profile", "standard"],
                                        "expect": {"rc": 0},
                                    },
                                ],
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )

            completed = run_harness(
                "--batch",
                str(batch),
                "--root",
                str(root),
                "--binary",
                str(fake_mcp),
                env={"E33_H02_SPAWN_MARKER": str(spawn_marker)},
            )

            with self.subTest("usage exit"):
                self.assertEqual(
                    2,
                    completed.returncode,
                    "a real+readonly batch containing shell/spawn mutators is invalid usage "
                    "and must be rejected by preflight\n"
                    f"stdout={completed.stdout}\nstderr={completed.stderr}",
                )
            with self.subTest("shell never ran"):
                self.assertFalse(shell_marker.exists(), "shell ran before the readonly preflight")
            with self.subTest("standard spawn never ran"):
                self.assertFalse(
                    spawn_marker.exists(), "standard-profile spawn ran before preflight"
                )

    def test_real_standard_batch_is_usage_error_before_any_step(self):
        with tempfile.TemporaryDirectory(prefix="lodestar-e33-h02-real-standard-") as temp:
            base = Path(temp)
            root = base / "real-root"
            root.mkdir()
            marker = root / "STEP_RAN"
            batch = base / "real-standard.json"
            batch.write_text(
                json.dumps(
                    {
                        "batch": "real_standard",
                        "root": "real",
                        "profile": "standard",
                        "cases": [
                            {
                                "id": "REAL-STANDARD",
                                "no_server": True,
                                "steps": [
                                    {
                                        "kind": "shell",
                                        "cmd": f"touch {shlex.quote(str(marker))}",
                                        "expect": {"rc": 0},
                                    }
                                ],
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )

            completed = run_harness(
                "--batch", str(batch), "--root", str(root), "--binary", sys.executable
            )

            self.assertEqual(2, completed.returncode, completed.stdout + completed.stderr)
            self.assertFalse(marker.exists(), "real+standard executed a step before rejection")

    def test_shell_tokens_preserve_metacharacter_paths_as_single_arguments(self):
        """Rendered @bin.mcp/@root values remain data, never shell syntax."""
        harness = load_harness_module("shell_token_quoting")
        with tempfile.TemporaryDirectory(prefix="lodestar-e33-h02-shell-") as temp:
            base = Path(temp)
            root = base / "root with spaces $(touch INJECTED_BY_ROOT)"
            root.mkdir()
            binary = base / "mcp with spaces $(touch INJECTED_BY_BIN)"
            binary.write_text("not executed\n", encoding="utf-8")
            entorno = harness.Entorno(binario=str(binary), root_real=str(root))
            command = (
                f"{shlex.quote(sys.executable)} -c "
                "'import json,sys; print(json.dumps(sys.argv[1:]))' "
                "@bin.mcp @root"
            )

            result = harness.run_step(
                {"kind": "shell", "cmd": command}, None, str(root), [], entorno
            )
            rendered_arguments = json.loads(result["stdout"].strip())

            with self.subTest("tokens are each one argument"):
                self.assertEqual([str(binary), str(root)], rendered_arguments)
            with self.subTest("@bin.mcp cannot inject shell syntax"):
                self.assertFalse((root / "INJECTED_BY_BIN").exists())
            with self.subTest("@root cannot inject shell syntax"):
                self.assertFalse((root / "INJECTED_BY_ROOT").exists())

    def test_binary_precedence_and_release_fallbacks_are_distinguishable(self):
        harness = load_harness_module("binary_precedence")
        expected_mcp_fallback = REPO_ROOT / "target" / "release" / "lodestar-mcp"
        expected_cli_fallback = REPO_ROOT / "target" / "release" / "lodestar"
        with mock.patch.dict(os.environ, {}, clear=True):
            fallback = harness.Entorno()
        with mock.patch.dict(
            os.environ,
            {"LODESTAR_MCP_BIN": "/env/lodestar-mcp", "LODESTAR_CLI_BIN": "/env/lodestar"},
            clear=True,
        ):
            from_environment = harness.Entorno()
            explicit = harness.Entorno(
                binario="/flag/lodestar-mcp", binario_cli="/flag/lodestar"
            )

        expectations = {
            "mcp fallback": (str(expected_mcp_fallback), fallback.binario),
            "cli fallback": (str(expected_cli_fallback), fallback.binario_cli),
            "mcp environment": ("/env/lodestar-mcp", from_environment.binario),
            "cli environment": ("/env/lodestar", from_environment.binario_cli),
            "mcp explicit flag": ("/flag/lodestar-mcp", explicit.binario),
            "cli explicit flag": ("/flag/lodestar", explicit.binario_cli),
        }
        for label, (expected, actual) in expectations.items():
            with self.subTest(label):
                self.assertEqual(expected, actual)

    def test_missing_explicit_cli_binary_is_execution_error_not_case_verdict(self):
        with tempfile.TemporaryDirectory(prefix="lodestar-e33-h02-cli-missing-") as temp:
            base = Path(temp)
            corpus = base / "corpus"
            corpus.mkdir()
            batch = base / "cli-missing.json"
            missing_cli = base / "does-not-exist" / "lodestar"
            batch.write_text(
                json.dumps(
                    {
                        "batch": "cli_missing",
                        "root": "corpus",
                        "profile": "readonly",
                        "cases": [
                            {
                                "id": "CLI-MISSING",
                                "no_server": True,
                                "steps": [
                                    {
                                        "kind": "shell",
                                        "cmd": "@bin.cli check",
                                        "expect": {"rc": 0},
                                    }
                                ],
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )

            completed = run_harness(
                "--batch",
                str(batch),
                "--root-corpus",
                str(corpus),
                "--binary",
                sys.executable,
                "--binary-cli",
                str(missing_cli),
            )

            self.assertEqual(
                3,
                completed.returncode,
                "an unavailable @bin.cli is a harness execution error, not FAIL/EXPLOR\n"
                f"stdout={completed.stdout}\nstderr={completed.stderr}",
            )
            self.assertIn("ERROR DE EJECUCIÓN", completed.stderr)
            self.assertNotIn("FAIL ", completed.stdout)
            self.assertNotIn("EXPLOR ", completed.stdout)

    def test_out_with_missing_parent_is_execution_error_without_traceback(self):
        with tempfile.TemporaryDirectory(prefix="lodestar-e33-h02-out-") as temp:
            base = Path(temp)
            corpus = base / "corpus"
            corpus.mkdir()
            batch = base / "empty.json"
            batch.write_text(
                json.dumps(
                    {
                        "batch": "empty",
                        "root": "corpus",
                        "profile": "readonly",
                        "cases": [],
                    }
                ),
                encoding="utf-8",
            )
            output = base / "missing-parent" / "results.json"

            completed = run_harness(
                "--batch",
                str(batch),
                "--root-corpus",
                str(corpus),
                "--binary",
                sys.executable,
                "--out",
                str(output),
            )

            with self.subTest("execution-error exit"):
                self.assertEqual(3, completed.returncode, completed.stdout + completed.stderr)
            with self.subTest("diagnostic is controlled"):
                self.assertNotIn("Traceback", completed.stderr)
            with self.subTest("not a case FAIL"):
                self.assertNotIn("FAIL ", completed.stdout)
            with self.subTest("no partial output"):
                self.assertFalse(output.exists())

    def test_malformed_step_expect_is_usage_error_before_execution(self):
        with tempfile.TemporaryDirectory(prefix="lodestar-e33-h02-expect-type-") as temp:
            base = Path(temp)
            corpus = base / "corpus"
            corpus.mkdir()
            marker = base / "MALFORMED_EXPECT_STEP_RAN"
            batch = base / "malformed-expect.json"
            batch.write_text(
                json.dumps(
                    {
                        "batch": "malformed_expect",
                        "root": "corpus",
                        "profile": "readonly",
                        "cases": [
                            {
                                "id": "MALFORMED-EXPECT",
                                "no_server": True,
                                "steps": [
                                    {
                                        "kind": "shell",
                                        "cmd": f"touch {shlex.quote(str(marker))}",
                                        "expect": [{"rc": 0}],
                                    }
                                ],
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )

            completed = run_harness(
                "--batch",
                str(batch),
                "--root-corpus",
                str(corpus),
                "--binary",
                sys.executable,
            )

            with self.subTest("usage exit"):
                self.assertEqual(
                    2,
                    completed.returncode,
                    "step.expect with list type must be rejected as invalid usage, not "
                    "downgraded to EXPLORATORY\n"
                    f"stdout={completed.stdout}\nstderr={completed.stderr}",
                )
            with self.subTest("step never ran"):
                self.assertFalse(marker.exists(), "malformed expect was detected after execution")


class CanonicalGateStructureTest(unittest.TestCase):
    def test_testbench_contains_no_machine_specific_absolute_path_variants(self):
        patterns = {
            "macOS user home": re.compile(r"/Users/[^/\s'\"]+/"),
            "Linux user home": re.compile(r"/home/[^/\s'\"]+/"),
            "private Claude scratchpad": re.compile(r"/private/tmp/claude-[^\s'\"]+"),
            "Windows user home": re.compile(r"[A-Za-z]:\\\\Users\\\\[^\\\s'\"]+"),
        }
        scanned = []
        violations = []
        for path in sorted(HARNESS.parent.rglob("*")):
            if (
                not path.is_file()
                or "__pycache__" in path.parts
                or path.suffix in {".pyc", ".pyo"}
            ):
                continue
            scanned.append(path.relative_to(REPO_ROOT).as_posix())
            text = path.read_text(encoding="utf-8", errors="replace")
            for label, pattern in patterns.items():
                for match in pattern.finditer(text):
                    violations.append(
                        f"{path.relative_to(REPO_ROOT)}: {label}: {match.group(0)}"
                    )

        self.assertIn(
            "docs/qa/testbench/build_artifact.py",
            scanned,
            "anti-vacuity: the portability scan skipped build_artifact.py",
        )
        self.assertEqual(
            [],
            violations,
            "E33-H02 removes machine-specific paths in every spelling, including "
            "scratchpad paths that merely encode `Users` with hyphens",
        )

    def test_build_artifact_runs_from_arbitrary_cwd_with_explicit_repo_inputs_and_temp_output(self):
        self.assertTrue(BUILD_ARTIFACT.is_file(), "anti-vacuity: build_artifact.py is missing")
        input_files = [HARNESS.parent / f"matriz_r{round_number}.json" for round_number in (1, 2, 3)]
        self.assertTrue(
            all(path.is_file() for path in input_files),
            f"anti-vacuity: repository artifact inputs are incomplete: {input_files}",
        )
        repo_html_before = {
            path.relative_to(REPO_ROOT).as_posix(): path.read_bytes()
            for path in HARNESS.parent.rglob("*.html")
        }

        with tempfile.TemporaryDirectory(prefix="lodestar-e33-h02-artifact-") as temp:
            base = Path(temp)
            arbitrary_cwd = base / "unrelated" / "cwd"
            arbitrary_cwd.mkdir(parents=True)
            output = base / "out" / "artifact.html"
            output.parent.mkdir()
            completed = subprocess.run(
                [
                    sys.executable,
                    str(BUILD_ARTIFACT),
                    "--input-dir",
                    str(HARNESS.parent),
                    "--out",
                    str(output),
                ],
                cwd=arbitrary_cwd,
                capture_output=True,
                text=True,
                timeout=30,
                check=False,
            )

            self.assertEqual(
                0,
                completed.returncode,
                "build_artifact must consume declared/repository inputs independent of cwd "
                "and must not consult a private scratchpad\n"
                f"stdout={completed.stdout}\nstderr={completed.stderr}",
            )
            self.assertTrue(output.is_file(), "the requested temporary artifact was not written")
            rendered = output.read_text(encoding="utf-8")
            self.assertIn("<title>", rendered, "anti-vacuity: output is not the HTML artifact")
            self.assertIn("TYP-01", rendered, "anti-vacuity: matrix input was not embedded")

        repo_html_after = {
            path.relative_to(REPO_ROOT).as_posix(): path.read_bytes()
            for path in HARNESS.parent.rglob("*.html")
        }
        self.assertEqual(
            repo_html_before,
            repo_html_after,
            "an explicit temporary --out must not create or overwrite an artifact in the repo",
        )

    def test_placeholder_and_expect_selectors_share_nonnegative_list_index_dialect(self):
        harness = load_harness_module("shared_list_path_dialect")
        step_results = [{"items": ["first", "second"]}]

        self.assertEqual(
            "second",
            harness.resolve_placeholders("@step0.items.1", step_results),
            "anti-vacuity: a valid positive placeholder index stopped resolving",
        )
        self.assertEqual(
            "second",
            harness.rx.resuelve_path(step_results[0], "items.1"),
            "anti-vacuity: a valid positive expect index stopped resolving",
        )
        self.assertIs(
            harness.rx.NO_RESUELVE,
            harness.rx.resuelve_path(step_results[0], "items.-1"),
            "expect paths must reject Python's negative-list-index extension",
        )
        with self.assertRaises(
            (IndexError, KeyError, TypeError, ValueError),
            msg="placeholders must reject negative indices just like expect paths",
        ):
            harness.resolve_placeholders("@step0.items.-1", step_results)

    def test_gate_inherits_batch_default_with_case_override_precedence(self):
        harness = load_harness_module("gate_inheritance")
        cases = [
            ({}, {"gate": False}, False),
            ({"gate": True}, {"gate": False}, True),
            ({"gate": False}, {"gate": True}, False),
            ({}, {}, True),
        ]

        self.assertEqual(4, len(cases), "anti-vacuity: the complete precedence table is required")
        actual = [harness.rx.entra_al_gate(case, spec) for case, spec, _ in cases]

        self.assertEqual(
            [expected for _, _, expected in cases],
            actual,
            "case gate must override batch gate, which otherwise inherits and defaults true",
        )

    def test_out_of_range_positive_index_is_absent_not_json_null_in_shared_dialect(self):
        harness = load_harness_module("out_of_range_positive_index")
        result = {"items": []}
        path = "items.0"

        self.assertEqual([], result["items"], "anti-vacuity: index zero must be out of range")
        self.assertIs(
            harness.rx.NO_RESUELVE,
            harness.rx.resuelve_path(result, path),
            "an out-of-range positive index must use the missing-path sentinel, not JSON null",
        )

        present_failures = harness.rx.evalua_paso(result, {"present": [path]}, 0)
        equals_null_failures = harness.rx.evalua_paso(
            result, {"equals": {path: None}}, 0
        )
        absent_failures = harness.rx.evalua_paso(result, {"absent": [path]}, 0)

        self.assertTrue(present_failures, "present must fail for an out-of-range index")
        self.assertIn(path, [failure["path"] for failure in present_failures])
        self.assertTrue(
            equals_null_failures,
            "equals:null must not confuse an out-of-range index with a resolved JSON null",
        )
        self.assertIn(path, [failure["path"] for failure in equals_null_failures])
        self.assertEqual([], absent_failures, "absent must pass for an out-of-range index")
        with self.assertRaises(
            ValueError,
            msg="placeholders must reject the same out-of-range selector",
        ):
            harness.resolve_placeholders("@step0.items.0", [result])

    def test_all_gate_shell_commands_are_posix_and_v_g2_04_uses_a_python_helper(self):
        harness = load_harness_module("gate_posix_shell")
        commands = []
        retention_case = None
        for relative_batch in harness.LOTES_DEL_GATE:
            spec = json.loads((HARNESS.parent / relative_batch).read_text(encoding="utf-8"))
            for case in spec["cases"]:
                if case["id"] == "V-G2-04-RETENCION-GC":
                    retention_case = case
                for step_index, step in enumerate(case.get("steps", [])):
                    if step.get("kind") == "shell":
                        commands.append((relative_batch, case["id"], step_index, step["cmd"]))

        self.assertTrue(commands, "anti-vacuity: no gate shell commands were inspected")
        ansi_c_quoting = [
            f"{batch}:{case_id}:step{step_index}"
            for batch, case_id, step_index, command in commands
            if re.search(r"(?:^|\s)\$'", command)
        ]
        self.assertEqual(
            [],
            ansi_c_quoting,
            "gate shell steps run through POSIX /bin/sh and cannot use Bash ANSI-C $'…' quoting",
        )

        self.assertIsNotNone(retention_case, "anti-vacuity: V-G2-04 is absent from the gate")
        probes = [
            step
            for step in retention_case["steps"]
            if step.get("kind") == "shell"
            and step.get("expect", {}).get("contains", {}).get("stdout") == "COUNT=1"
            and step.get("expect", {}).get("matches", {}).get("stdout") == "LATEST_MATCH=True"
        ]
        self.assertEqual(
            1,
            len(probes),
            "V-G2-04 must retain one explicit COUNT=1/LATEST_MATCH observation",
        )
        tokens = shlex.split(probes[0]["cmd"], posix=True)
        self.assertTrue(tokens, "anti-vacuity: V-G2-04 probe command is empty")
        self.assertRegex(Path(tokens[0]).name, r"^python3(?:\.\d+)?$")
        self.assertTrue(
            any(token.startswith("@testbench/") and token.endswith(".py") for token in tokens),
            "the portable V-G2-04 probe must invoke a checked-in Python helper by @testbench path",
        )
        self.assertIn("@root", tokens)
        self.assertIn("@bin.mcp", tokens)

    def test_verify_g1_12_is_ported_as_zero_backlinks_plan_apply_and_material_delete(self):
        spec = json.loads((BATCHES / "gate_L6_plan.json").read_text(encoding="utf-8"))
        target = "fixtures/sin-entrantes.md"
        qualifying = []
        diagnostics = []
        for case in spec["cases"]:
            steps = case.get("steps", [])
            backlink_indices = []
            plan_indices = []
            apply_indices = []
            delete_indices = []
            for index, step in enumerate(steps):
                arguments = step.get("arguments", {})
                expect = step.get("expect", {})
                equals = expect.get("equals", {})
                if (
                    step.get("tool") == "graph_query"
                    and arguments.get("operation") == "backlinks"
                    and arguments.get("ref", {}).get("path") == target
                    and equals.get("structured.summary.edgeCount") == 0
                ):
                    backlink_indices.append(index)

                operations = arguments.get("operations", [])
                if step.get("tool") == "change_plan" and len(operations) == 1:
                    operation = operations[0]
                    normalized_default = (
                        equals.get(
                            "structured.normalizedOperations.0.inbound_links_policy"
                        )
                        or equals.get(
                            "structured.normalizedOperations.0.inboundLinksPolicy"
                        )
                    )
                    if (
                        operation.get("op") == "delete"
                        and operation.get("path") == target
                        and "inboundLinksPolicy" not in operation
                        and expect.get("is_error") is False
                        and equals.get("structured.canApply") is True
                        and normalized_default == "reject"
                    ):
                        plan_indices.append(index)

                change_set = arguments.get("changeSetId")
                if (
                    step.get("tool") == "change_apply"
                    and equals.get("structured.applied") is True
                    and isinstance(change_set, str)
                    and re.fullmatch(r"@step\d+\.structured\.changeSetId", change_set)
                ):
                    apply_indices.append(index)

                command = step.get("cmd", "")
                deletion_observed = (
                    expect.get("contains", {}).get("stdout") == "BORRADO"
                    or expect.get("matches", {}).get("stdout") in {
                        "^BORRADO\\n?$",
                        "BORRADO",
                    }
                    or (expect.get("rc") == 0 and "test ! -e" in command)
                )
                if step.get("kind") == "shell" and target in command and deletion_observed:
                    delete_indices.append(index)

            flows = [
                (backlinks, plan, apply, deleted)
                for backlinks in backlink_indices
                for plan in plan_indices
                for apply in apply_indices
                for deleted in delete_indices
                if backlinks < plan < apply < deleted
                and steps[apply]["arguments"]["changeSetId"]
                == f"@step{plan}.structured.changeSetId"
            ]
            if flows and case.get("fresh_root") is True:
                qualifying.append((case["id"], flows[0]))
            elif any((backlink_indices, plan_indices, apply_indices, delete_indices)):
                diagnostics.append(
                    f"{case['id']}: backlinks={backlink_indices}, plan={plan_indices}, "
                    f"apply={apply_indices}, delete={delete_indices}, "
                    f"fresh_root={case.get('fresh_root')}"
                )

        self.assertEqual(
            1,
            len(qualifying),
            "verify_G1-12 must live in one existing fresh-root L6 gate case and prove, "
            "in order: zero backlinks; omitted input policy with observable normalized "
            "default reject; applicable plan; apply; and material deletion. "
            f"Partial candidates: {diagnostics}",
        )

    def test_gate_verify_g1_claims_g1_12_is_delete_without_inbound_links(self):
        spec = json.loads((BATCHES / "gate_verify_g1.json").read_text(encoding="utf-8"))
        description = spec.get("descripcion", "")
        self.assertIn("G1-12", description, "anti-vacuity: the porting claim disappeared")
        self.assertNotRegex(
            description,
            r"G1-12[^·\n]*(?:delete|borrado)[^·\n]*con entrantes",
            "G1-12 is the zero-backlink delete repro, not the separate inbound-link guard",
        )
        self.assertRegex(
            description,
            r"G1-12[^·\n]*(?:sin entrantes|sin backlinks|0 (?:entrantes|backlinks))",
            "the structural claim must identify G1-12's distinguishing zero-backlink premise",
        )

    def test_h5_nested_harness_calls_forward_the_selected_mcp_binary(self):
        spec = json.loads((BATCHES / "H5_cursor_cli.json").read_text(encoding="utf-8"))
        case = next(case for case in spec["cases"] if case["id"] == "G2-05")
        nested_calls = []
        invocation = re.compile(
            r"python3\s+lodestar_harness\.py(?P<args>.*?)"
            r"(?=\s*&&\s*python3\s+lodestar_harness\.py|$)"
        )
        for step in case["steps"]:
            nested_calls.extend(match.group("args") for match in invocation.finditer(step["cmd"]))

        self.assertEqual(5, len(nested_calls), "anti-vacuity: H5 nested call inventory changed")
        missing_binary = [
            index
            for index, arguments in enumerate(nested_calls)
            if re.search(r"(?:^|\s)--binary\s+@bin\.mcp(?:\s|$)", arguments) is None
        ]
        self.assertEqual(
            [],
            missing_binary,
            "each fresh nested harness process must inherit the outer selected binary",
        )

    def test_gate_l1_typ_09_does_not_equals_filesystem_dependent_total(self):
        spec = json.loads((BATCHES / "gate_L1_consulta.json").read_text(encoding="utf-8"))
        case = next(case for case in spec["cases"] if case["id"] == "L1-TYP-09")
        expect = case["steps"][0]["expect"]

        self.assertTrue(
            any(key != "describe" for key in expect),
            "anti-vacuity: the canonical gate case must remain assertable",
        )
        self.assertNotIn(
            "structured.totalApproximate",
            expect.get("equals", {}),
            "FORMATO_EXPECT §6 forbids equals on filesystem-dependent totalApproximate",
        )

    def test_gate_l2_absence_assertion_targets_outgoing_links(self):
        spec = json.loads((BATCHES / "gate_L2_proyeccion.json").read_text(encoding="utf-8"))
        case = next(case for case in spec["cases"] if case["id"] == "L2-PRJ-05")
        absent = case["steps"][0]["expect"]["absent"]

        self.assertIn("structured.document.outgoingLinks", absent)
        self.assertNotIn(
            "structured.document.out",
            absent,
            "`out` belongs below backlinks; it cannot prove outgoingLinks was omitted",
        )

    def test_gate_h_json_and_sarif_assert_the_real_cli_exit_while_validating_payload(self):
        harness = load_harness_module("gate_h_cli_exit_observable")
        spec = json.loads(
            (BATCHES / "gate_H_cli_recuperacion.json").read_text(encoding="utf-8")
        )
        case = next(case for case in spec["cases"] if case["id"] == "H-CLI-EXIT-CODES")
        format_steps = {
            output_format: next(
                step
                for step in case["steps"]
                if f"--{output_format}" in step.get("cmd", "")
                and not ("--json" in step.get("cmd", "") and "--sarif" in step.get("cmd", ""))
            )
            for output_format in ("json", "sarif")
        }

        with tempfile.TemporaryDirectory(prefix="lodestar-e33-h02-cli-wire-") as temp:
            base = Path(temp)
            root = base / "root"
            root.mkdir()
            fake_cli = base / "fake-lodestar"
            fake_cli.write_text(
                "#!/usr/bin/env python3\n"
                "import json, os, sys\n"
                "if '--sarif' in sys.argv:\n"
                "    payload = {'version': '2.1.0', 'runs': [{'tool': {'driver': "
                "{'name': 'lodestar'}}, 'results': [{'ruleId': 'LINK-TARGET-MISSING'}]}]}\n"
                "else:\n"
                "    payload = {'dangling': 1, 'diagnostics': [], 'documents': 1, "
                "'incoming': 0, 'isolated': 0, 'outgoing': 0, "
                "'recoveryPending': False, 'valid': False}\n"
                "json.dump(payload, sys.stdout)\n"
                "sys.exit(int(os.environ['E33_H02_FAKE_CLI_EXIT']))\n",
                encoding="utf-8",
            )
            fake_cli.chmod(0o755)
            entorno = harness.Entorno(binario=sys.executable, binario_cli=str(fake_cli))

            for output_format, step in format_steps.items():
                with self.subTest(output_format=output_format, real_exit=1):
                    with mock.patch.dict(os.environ, {"E33_H02_FAKE_CLI_EXIT": "1"}):
                        exit_one = harness.run_step(step, None, str(root), [], entorno)
                    self.assertEqual(
                        [],
                        harness.rx.evalua_paso(exit_one, step["expect"], 0),
                        "anti-vacuity: a valid payload plus the required CLI exit 1 "
                        f"must satisfy the {output_format} probe; result={exit_one}",
                    )

                with self.subTest(output_format=output_format, real_exit=0):
                    with mock.patch.dict(os.environ, {"E33_H02_FAKE_CLI_EXIT": "0"}):
                        exit_zero = harness.run_step(step, None, str(root), [], entorno)
                    self.assertTrue(
                        harness.rx.evalua_paso(exit_zero, step["expect"], 0),
                        f"the {output_format} probe validated the payload but lost the "
                        "real CLI exit: changing only exit 1 to 0 remained a PASS",
                    )

    def test_gate_h_contains_an_observable_cli_runtime_exit_three_probe(self):
        spec = json.loads(
            (BATCHES / "gate_H_cli_recuperacion.json").read_text(encoding="utf-8")
        )
        case = next(case for case in spec["cases"] if case["id"] == "H-CLI-EXIT-CODES")
        cli_steps = [step for step in case["steps"] if "@bin.cli" in step.get("cmd", "")]
        self.assertTrue(cli_steps, "anti-vacuity: H-CLI-EXIT-CODES has no CLI probes")

        exit_three_steps = []
        for step in cli_steps:
            expect = step.get("expect", {})
            asserted_values = json.dumps(
                {key: value for key, value in expect.items() if key != "describe"},
                ensure_ascii=False,
            )
            if expect.get("rc") == 3 or re.search(r"EXIT(?:_[A-Z]+)?=3\b", asserted_values):
                exit_three_steps.append(step)

        self.assertTrue(
            exit_three_steps,
            "the frozen CLI exit-code case covers 0/1/2 but has no observable CLI "
            "runtime/IO exit 3 assertion",
        )

    def test_v_g2_04_reopens_after_retention_then_applies_and_keeps_latest_receipt(self):
        spec = json.loads((BATCHES / "gate_verify_g2.json").read_text(encoding="utf-8"))
        case = next(case for case in spec["cases"] if case["id"] == "V-G2-04-RETENCION-GC")
        steps = case["steps"]
        config_index = next(
            index
            for index, step in enumerate(steps)
            if "maximumReceipts" in step.get("cmd", "")
        )
        tail = steps[config_index + 1 :]
        self.assertTrue(tail, "anti-vacuity: the case ends immediately after writing retention")

        session_boundaries = []
        for offset, step in enumerate(tail, start=config_index + 1):
            command = step.get("cmd", "")
            explicit_restart = step.get("new_session") is True or step.get("kind") in {
                "restart_session",
                "session_restart",
            }
            separate_process = step.get("kind") == "shell" and (
                "lodestar_harness.py" in command or "@bin.mcp" in command
            )
            if explicit_restart or separate_process:
                session_boundaries.append(offset)
        self.assertTrue(
            session_boundaries,
            "maximumReceipts is session-scoped: V-G2-04 must open a new MCP session "
            "after writing it",
        )

        first_boundary = session_boundaries[0]
        boundary_step = steps[first_boundary]
        explicit_restart = boundary_step.get("new_session") is True or boundary_step.get(
            "kind"
        ) in {"restart_session", "session_restart"}
        if explicit_restart:
            observed_steps = steps[first_boundary + 1 :]
            application_in_new_session = any(
                step.get("tool") == "change_apply" for step in observed_steps
            )
        else:
            # A shell-launched process is a new session only if that process owns the
            # application (direct JSON-RPC text or a nested batch). A later outer
            # `call` would still use the old, config-cached session.
            observed_steps = [boundary_step]
            boundary_command = boundary_step.get("cmd", "")
            application_in_new_session = (
                "change_apply" in boundary_command or "--batch" in boundary_command
            )
        serialized_observation = json.dumps(observed_steps, ensure_ascii=False)
        self.assertTrue(
            application_in_new_session,
            "the reopened session must perform an apply so receipt GC actually runs",
        )

        count_is_asserted = any(
            step.get("expect", {}).get("length", {}).get("structured.receipts") == 1
            for step in observed_steps
        ) or "COUNT=1" in serialized_observation
        latest_is_asserted = bool(
            re.search(r"(?:LATEST|RECENT|MAS_RECIENTE)=1", serialized_observation)
        ) or (
            "structured.receipts.0.receiptId" in serialized_observation
            and "structured.receiptId" in serialized_observation
        )
        self.assertTrue(count_is_asserted, "the post-GC observation must assert COUNT=1")
        self.assertTrue(
            latest_is_asserted,
            "COUNT=1 alone is insufficient: V-G2-04 must prove the retained receipt "
            "is the one produced by the latest apply",
        )

    def test_readme_invariant_example_references_only_existing_step_indices(self):
        readme = (HARNESS.parent / "README.md").read_text(encoding="utf-8")
        examples = [
            json.loads(match.group("body"))
            for match in re.finditer(
                r"```jsonc\s*\n(?P<body>.*?)\n```", readme, flags=re.DOTALL
            )
            if '"invariant"' in match.group("body")
        ]
        self.assertTrue(examples, "anti-vacuity: README has no invariant JSON example")

        violations = []
        for example in examples:
            step_count = len(example.get("steps", []))
            for invariant in example.get("expect", []):
                for index in invariant.get("steps", []):
                    if not isinstance(index, int) or isinstance(index, bool) or not 0 <= index < step_count:
                        violations.append(
                            f"{example.get('id')}: step {index} does not exist "
                            f"(example has {step_count} step(s))"
                        )
        self.assertEqual([], violations, "README must teach an executable invariant")

    def test_bdd_3_gate_inventory_has_16_batches_and_97_unique_assertable_cases(self):
        harness = load_harness_module("gate_inventory")
        expected_batches = [
            "batches/gate_L1_consulta.json",
            "batches/gate_L2_proyeccion.json",
            "batches/gate_L3_metadata.json",
            "batches/gate_L5_grafo.json",
            "batches/gate_L6_plan.json",
            "batches/gate_L7_apply.json",
            "batches/gate_L8_readonly.json",
            "batches/gate_L9_check_a.json",
            "batches/gate_L10_check_b.json",
            "batches/gate_L11_scopes.json",
            "batches/gate_L12_robustez.json",
            "batches/gate_G_descubrimiento.json",
            "batches/gate_H_cli_recuperacion.json",
            "batches/gate_invariantes.json",
            "batches/gate_verify_g1.json",
            "batches/gate_verify_g2.json",
        ]
        self.assertEqual(expected_batches, harness.LOTES_DEL_GATE)

        cases = []
        for relative_batch in harness.LOTES_DEL_GATE:
            spec = json.loads((HARNESS.parent / relative_batch).read_text(encoding="utf-8"))
            for case in spec["cases"]:
                cases.append((relative_batch, spec, case))

        ids = [case["id"] for _, _, case in cases]
        self.assertEqual(97, len(ids), "BDD-3 canonical gate case count was reduced")
        self.assertEqual(97, len(set(ids)), "gate case ids must be globally unique")
        non_assertable = [
            f"{batch}:{case['id']}"
            for batch, _, case in cases
            if not harness.rx.es_asertable(case)
        ]
        demos = [
            f"{batch}:{case['id']}"
            for batch, spec, case in cases
            if not harness.rx.entra_al_gate(case, spec)
        ]
        self.assertEqual([], non_assertable, "every gate case must carry a real assertion")
        self.assertEqual([], demos, "LOTES_DEL_GATE must not hide gate:false demos")


if __name__ == "__main__":
    unittest.main()
