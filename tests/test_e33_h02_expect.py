"""Contract tests for every E33-H02 ``expect`` assertion family."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import unittest


REPO_ROOT = Path(__file__).resolve().parents[1]
EXPECT_MODULE = REPO_ROOT / "docs" / "qa" / "testbench" / "runner_expect.py"


def load_expect_module():
    spec = importlib.util.spec_from_file_location("e33_h02_runner_expect", EXPECT_MODULE)
    if spec is None or spec.loader is None:
        raise AssertionError(f"could not load expect evaluator from {EXPECT_MODULE}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ExpectFamilyMatrixTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.rx = load_expect_module()

    def test_each_step_expect_family_accepts_match_and_rejects_false_value_with_path(self):
        structured_result = {
            "structured": {
                "value": 7,
                "present": None,
                "words": "alpha beta",
                "items": ["alpha", {"nested": True}],
                "object": {"key": "value"},
            },
            "rc": 4,
        }
        rows = [
            (
                "is_error",
                {"is_error": True},
                {"is_error": True},
                {"is_error": False},
                "is_error",
            ),
            (
                "error_code",
                {"is_error": True, "error_code": "NOT_FOUND"},
                {"error_code": "NOT_FOUND"},
                {"error_code": "OTHER"},
                "error_code",
            ),
            (
                "protocol_error_code",
                {"protocol_error": {"code": -32602}},
                {"protocol_error_code": -32602},
                {"protocol_error_code": -32603},
                "protocol_error.code",
            ),
            (
                "equals",
                structured_result,
                {"equals": {"structured.value": 7}},
                {"equals": {"structured.value": 8}},
                "structured.value",
            ),
            (
                "present",
                structured_result,
                {"present": ["structured.present"]},
                {"present": ["structured.missing"]},
                "structured.missing",
            ),
            (
                "absent",
                structured_result,
                {"absent": ["structured.missing"]},
                {"absent": ["structured.value"]},
                "structured.value",
            ),
            (
                "matches",
                structured_result,
                {"matches": {"structured.words": r"^alpha"}},
                {"matches": {"structured.words": r"^omega"}},
                "structured.words",
            ),
            (
                "contains",
                structured_result,
                {"contains": {"structured.items": "alpha"}},
                {"contains": {"structured.items": "omega"}},
                "structured.items",
            ),
            (
                "not_contains",
                structured_result,
                {"not_contains": {"structured.words": "omega"}},
                {"not_contains": {"structured.words": "alpha"}},
                "structured.words",
            ),
            (
                "length",
                structured_result,
                {"length": {"structured.items": 2}},
                {"length": {"structured.items": 3}},
                "structured.items",
            ),
            (
                "min_length",
                structured_result,
                {"min_length": {"structured.items": 2}},
                {"min_length": {"structured.items": 3}},
                "structured.items",
            ),
            (
                "type",
                structured_result,
                {"type": {"structured.object": "object"}},
                {"type": {"structured.object": "array"}},
                "structured.object",
            ),
            ("rc", structured_result, {"rc": 4}, {"rc": 5}, "rc"),
        ]

        for family, result, matching, deliberately_false, expected_path in rows:
            with self.subTest(family=family, outcome="match"):
                self.assertEqual([], self.rx.evalua_paso(result, matching, 0))
            with self.subTest(family=family, outcome="false"):
                failures = self.rx.evalua_paso(result, deliberately_false, 0)
                self.assertTrue(failures, f"{family} false expectation was ignored")
                self.assertIn(expected_path, [failure["path"] for failure in failures])

    def test_same_and_differs_each_accept_match_and_reject_false_value_with_path(self):
        rows = [
            (
                "same",
                [{"structured": {"revision": "A"}}, {"structured": {"revision": "A"}}],
                [{"structured": {"revision": "A"}}, {"structured": {"revision": "B"}}],
            ),
            (
                "differs",
                [{"structured": {"revision": "A"}}, {"structured": {"revision": "B"}}],
                [{"structured": {"revision": "A"}}, {"structured": {"revision": "A"}}],
            ),
        ]
        for invariant, matching_results, false_results in rows:
            declaration = {
                "invariant": invariant,
                "steps": [0, 1],
                "path": "structured.revision",
            }
            with self.subTest(invariant=invariant, outcome="match"):
                self.assertEqual([], self.rx.evalua_invariante(matching_results, declaration))
            with self.subTest(invariant=invariant, outcome="false"):
                failures = self.rx.evalua_invariante(false_results, declaration)
                self.assertTrue(failures, f"{invariant} false invariant was ignored")
                self.assertIn("structured.revision", [failure["path"] for failure in failures])

    def test_same_and_differs_use_json_structural_equality_for_types_and_order(self):
        rows = [
            ("boolean exact", True, True, True),
            ("number exact", 1, 1, True),
            ("boolean versus number", True, 1, False),
            ("number versus boolean", 1, True, False),
            ("list same order", [1, 2], [1, 2], True),
            ("list opposite order", [1, 2], [2, 1], False),
            (
                "object same order",
                {"first": 1, "second": 2},
                {"first": 1, "second": 2},
                True,
            ),
            (
                "object opposite order",
                {"first": 1, "second": 2},
                {"second": 2, "first": 1},
                False,
            ),
        ]
        declaration = {
            "steps": [0, 1],
            "path": "structured.value",
        }

        for label, first, second, structurally_equal in rows:
            results = [
                {"structured": {"value": first}},
                {"structured": {"value": second}},
            ]
            for invariant in ("same", "differs"):
                with self.subTest(label=label, invariant=invariant):
                    failures = self.rx.evalua_invariante(
                        results,
                        {**declaration, "invariant": invariant},
                    )
                    must_pass = (
                        structurally_equal if invariant == "same" else not structurally_equal
                    )
                    if must_pass:
                        self.assertEqual(
                            [],
                            failures,
                            f"anti-vacuity: {invariant} rejected {label}",
                        )
                    else:
                        self.assertTrue(
                            failures,
                            f"{invariant} ignored JSON type/order inequality for {label}",
                        )
                        self.assertIn(
                            "structured.value",
                            [failure["path"] for failure in failures],
                        )

    def test_unknown_expect_key_is_a_named_failure(self):
        failures = self.rx.evalua_paso(
            {"structured": {"value": 7}}, {"equals_typo": {"structured.value": 7}}, 0
        )

        self.assertTrue(failures)
        self.assertIn("equals_typo", [failure["path"] for failure in failures])

    def test_harness_exception_cannot_pass_when_other_assertions_match(self):
        failures = self.rx.evalua_paso(
            {"harness_exception": "boom", "structured": {"value": 7}},
            {"equals": {"structured.value": 7}},
            0,
        )

        self.assertTrue(failures)
        self.assertIn("harness_exception", [failure["path"] for failure in failures])

    def test_error_code_requires_a_tool_error_even_when_the_code_matches(self):
        failures = self.rx.evalua_paso(
            {"is_error": False, "error_code": "INVALID_SCHEMA"},
            {"error_code": "INVALID_SCHEMA"},
            0,
        )

        self.assertTrue(
            failures,
            "error_code must imply is_error=true; a stale/misleading code cannot pass",
        )
        self.assertIn("error_code", [failure["path"] for failure in failures])
        self.assertTrue(
            any("is_error" in failure["reason"] for failure in failures),
            f"failure did not explain the tool-error requirement: {failures}",
        )

    def test_is_error_requires_an_exact_json_boolean_not_numeric_zero_or_one(self):
        controls = [
            ({"is_error": True}, {"is_error": True}),
            ({"is_error": False}, {"is_error": False}),
        ]
        for result, expectation in controls:
            with self.subTest(control=result):
                self.assertEqual([], self.rx.evalua_paso(result, expectation, 0))

        numeric_lookalikes = [
            ({"is_error": 1}, {"is_error": True}),
            ({"is_error": 0}, {"is_error": False}),
        ]
        for result, expectation in numeric_lookalikes:
            with self.subTest(result=result, expectation=expectation):
                failures = self.rx.evalua_paso(result, expectation, 0)
                self.assertTrue(
                    failures,
                    "FORMATO_EXPECT §3 requires the real is_error value to be the exact "
                    "JSON boolean, not a truthy/falsy number",
                )
                self.assertIn("is_error", [failure["path"] for failure in failures])

    def test_error_code_requires_is_error_to_be_exactly_true_not_numeric_one(self):
        failures = self.rx.evalua_paso(
            {"is_error": 1, "error_code": "INVALID_SCHEMA"},
            {"error_code": "INVALID_SCHEMA"},
            0,
        )

        self.assertTrue(
            failures,
            "error_code implies is_error with the exact JSON value true; integer 1 "
            "must not masquerade as a tool error",
        )
        self.assertIn("error_code", [failure["path"] for failure in failures])

    def test_integer_assertions_reject_boolean_actual_values(self):
        controls = [
            (
                {"protocol_error": {"code": -32602}},
                {"protocol_error_code": -32602},
            ),
            ({"rc": 1}, {"rc": 1}),
        ]
        for result, expectation in controls:
            with self.subTest(control=result):
                self.assertEqual([], self.rx.evalua_paso(result, expectation, 0))

        boolean_lookalikes = [
            ({"protocol_error": {"code": True}}, {"protocol_error_code": 1},
             "protocol_error.code"),
            ({"protocol_error": {"code": False}}, {"protocol_error_code": 0},
             "protocol_error.code"),
            ({"rc": True}, {"rc": 1}, "rc"),
            ({"rc": False}, {"rc": 0}, "rc"),
        ]
        for result, expectation, expected_path in boolean_lookalikes:
            with self.subTest(result=result, expectation=expectation):
                failures = self.rx.evalua_paso(result, expectation, 0)
                self.assertTrue(
                    failures,
                    "a real JSON boolean must not satisfy an integer assertion merely "
                    "because bool subclasses int in Python",
                )
                self.assertIn(expected_path, [failure["path"] for failure in failures])

    def test_equals_uses_json_types_and_never_equates_boolean_with_number(self):
        rows = [
            ({"structured": {"value": True}}, 1),
            ({"structured": {"value": False}}, 0),
            ({"structured": {"value": 1}}, True),
            ({"structured": {"value": 0}}, False),
            ({"structured": {"value": [True]}}, [1]),
            ({"structured": {"value": {"nested": False}}}, {"nested": 0}),
        ]

        for result, expected in rows:
            with self.subTest(result=result, expected=expected):
                failures = self.rx.evalua_paso(
                    result, {"equals": {"structured.value": expected}}, 0
                )
                self.assertTrue(
                    failures,
                    "JSON structural equality collapsed boolean and number types",
                )
                self.assertIn("structured.value", [failure["path"] for failure in failures])

    def test_contains_and_not_contains_use_json_types_without_equating_bool_and_number(self):
        rows = [
            ([True], True, 1),
            ([1], 1, True),
        ]

        for actual, matching, numeric_boolean_lookalike in rows:
            with self.subTest(actual=actual, expected=matching, outcome="match"):
                self.assertEqual(
                    [],
                    self.rx.evalua_paso(
                        {"structured": {"items": actual}},
                        {"contains": {"structured.items": matching}},
                        0,
                    ),
                    "anti-vacuity: contains must accept an element with the same JSON type",
                )
            with self.subTest(
                actual=actual,
                expected=numeric_boolean_lookalike,
                outcome="type mismatch",
            ):
                failures = self.rx.evalua_paso(
                    {"structured": {"items": actual}},
                    {"contains": {"structured.items": numeric_boolean_lookalike}},
                    0,
                )
                self.assertTrue(
                    failures,
                    "contains collapsed boolean and number types instead of using JSON "
                    "structural equality",
                )
                self.assertIn("structured.items", [failure["path"] for failure in failures])
            with self.subTest(
                actual=actual,
                expected=numeric_boolean_lookalike,
                family="not_contains",
                outcome="type mismatch",
            ):
                self.assertEqual(
                    [],
                    self.rx.evalua_paso(
                        {"structured": {"items": actual}},
                        {"not_contains": {"structured.items": numeric_boolean_lookalike}},
                        0,
                    ),
                    "anti-vacuity: not_contains must accept an absent JSON value of a "
                    "different type",
                )
            with self.subTest(
                actual=actual,
                expected=matching,
                family="not_contains",
                outcome="match",
            ):
                failures = self.rx.evalua_paso(
                    {"structured": {"items": actual}},
                    {"not_contains": {"structured.items": matching}},
                    0,
                )
                self.assertTrue(
                    failures,
                    "not_contains accepted an element with the same JSON value and type",
                )
                self.assertIn("structured.items", [failure["path"] for failure in failures])

    def test_equals_rejects_an_object_with_members_in_the_opposite_order(self):
        actual = {"first": 1, "second": 2}
        opposite_order = {"second": 2, "first": 1}
        self.assertEqual(
            ["first", "second"],
            list(actual),
            "anti-vacuity: the real object order was not constructed as intended",
        )
        self.assertEqual(
            ["second", "first"],
            list(opposite_order),
            "anti-vacuity: the expected object order was not reversed",
        )

        failures = self.rx.evalua_paso(
            {"structured": {"object": actual}},
            {"equals": {"structured.object": opposite_order}},
            0,
        )

        self.assertTrue(
            failures,
            "FORMATO_EXPECT §3 defines ordered structural equality for object members",
        )
        self.assertIn("structured.object", [failure["path"] for failure in failures])

    def test_equals_rejects_a_list_with_the_same_members_in_a_different_order(self):
        actual = [1, 2]
        reversed_order = [2, 1]
        self.assertCountEqual(
            actual,
            reversed_order,
            "anti-vacuity: both lists must contain the same members",
        )
        self.assertNotEqual(
            actual,
            reversed_order,
            "anti-vacuity: the two list orders accidentally match",
        )

        failures = self.rx.evalua_paso(
            {"structured": {"items": actual}},
            {"equals": {"structured.items": reversed_order}},
            0,
        )

        self.assertTrue(
            failures,
            "equals must compare JSON arrays in order, not as sets or multisets",
        )
        self.assertIn("structured.items", [failure["path"] for failure in failures])

    def test_not_contains_fails_when_the_selected_path_does_not_exist(self):
        result = {"structured": {"present_items": ["alpha"]}}
        self.assertIs(
            self.rx.NO_RESUELVE,
            self.rx.resuelve_path(result, "structured.missing_items"),
            "anti-vacuity: the negative control path unexpectedly resolves",
        )

        failures = self.rx.evalua_paso(
            result,
            {"not_contains": {"structured.missing_items": "omega"}},
            0,
        )

        self.assertTrue(
            failures,
            "not_contains cannot pass vacuously when its path does not resolve",
        )
        self.assertIn("structured.missing_items", [failure["path"] for failure in failures])

    def test_type_number_rejects_json_boolean(self):
        for value in (True, False):
            with self.subTest(value=value):
                failures = self.rx.evalua_paso(
                    {"structured": {"value": value}},
                    {"type": {"structured.value": "number"}},
                    0,
                )
                self.assertTrue(failures, "JSON booleans are not JSON numbers")
                self.assertIn("structured.value", [failure["path"] for failure in failures])

    def test_same_checks_every_declared_step_not_only_the_first_pair(self):
        declaration = {
            "invariant": "same",
            "steps": [0, 1, 2],
            "path": "structured.revision",
        }
        results = [
            {"structured": {"revision": "A"}},
            {"structured": {"revision": "A"}},
            {"structured": {"revision": "B"}},
        ]

        failures = self.rx.evalua_invariante(results, declaration)

        self.assertTrue(failures, "same ignored the differing third declared step")
        self.assertIn(2, [failure["step"] for failure in failures])
        self.assertIn("structured.revision", [failure["path"] for failure in failures])

    def test_differs_accepts_when_only_the_third_declared_step_is_distinct(self):
        declaration = {
            "invariant": "differs",
            "steps": [0, 1, 2],
            "path": "structured.revision",
        }
        results = [
            {"structured": {"revision": "A"}},
            {"structured": {"revision": "A"}},
            {"structured": {"revision": "B"}},
        ]

        self.assertEqual([], self.rx.evalua_invariante(results, declaration))

    def test_same_and_differs_require_the_path_in_every_declared_step(self):
        path = "structured.revision"
        rows = [
            (
                "same",
                [{"structured": {"revision": "A"}}, {"structured": {"revision": "A"}}],
                [
                    (
                        "missing in one step",
                        [{"structured": {"revision": "A"}}, {"structured": {}}],
                        [1],
                    ),
                    (
                        "missing in every step",
                        [{"structured": {}}, {"structured": {}}],
                        [0, 1],
                    ),
                ],
            ),
            (
                "differs",
                [{"structured": {"revision": "A"}}, {"structured": {"revision": "B"}}],
                [
                    (
                        "missing in one step",
                        [{"structured": {"revision": "A"}}, {"structured": {}}],
                        [1],
                    ),
                    (
                        "missing in every step",
                        [{"structured": {}}, {"structured": {}}],
                        [0, 1],
                    ),
                ],
            ),
        ]

        for invariant, present_results, missing_variants in rows:
            declaration = {"invariant": invariant, "steps": [0, 1], "path": path}
            with self.subTest(invariant=invariant, variant="path present"):
                self.assertEqual(
                    [],
                    self.rx.evalua_invariante(present_results, declaration),
                    "anti-vacuity: a valid invariant with the path present must pass",
                )
            for variant, results, guilty_steps in missing_variants:
                with self.subTest(invariant=invariant, variant=variant):
                    failures = self.rx.evalua_invariante(results, declaration)
                    self.assertTrue(
                        failures,
                        f"{invariant} passed even though {path} was {variant}",
                    )
                    self.assertEqual([path] * len(guilty_steps), [f["path"] for f in failures])
                    self.assertEqual(guilty_steps, [f["step"] for f in failures])

    def test_empty_or_describe_only_expect_remains_exploratory(self):
        rows = {
            "short empty": {"id": "EMPTY", "tool": "workspace_status", "expect": {}},
            "short describe": {
                "id": "DESCRIBE",
                "tool": "workspace_status",
                "expect": {"describe": "prose is not an assertion"},
            },
            "long step empty": {
                "id": "STEP-EMPTY",
                "steps": [{"kind": "call", "tool": "workspace_status", "expect": {}}],
            },
            "long step describe": {
                "id": "STEP-DESCRIBE",
                "steps": [
                    {
                        "kind": "call",
                        "tool": "workspace_status",
                        "expect": {"describe": "prose is not an assertion"},
                    }
                ],
            },
        }

        for label, case in rows.items():
            with self.subTest(label=label):
                self.assertFalse(
                    self.rx.es_asertable(case),
                    "an empty/describe-only expect must not become a vacuous PASS",
                )
                verdict, failures = self.rx.evalua_caso(case, [{"is_error": False}])
                self.assertEqual(self.rx.EXPLORATORY, verdict)
                self.assertEqual([], failures)


if __name__ == "__main__":
    unittest.main()
