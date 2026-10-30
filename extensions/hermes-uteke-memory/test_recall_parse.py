"""Run: python3 -m unittest discover -s extensions/hermes-uteke-memory -p 'test_*.py'

Checks the Hermes plugin against the real `uteke recall --json` output
(fixture) with default settings (#1593).
"""
import importlib.machinery
import importlib.util
import json
import os
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURE = os.path.join(HERE, "..", "fixtures", "recall-cli.json")


def load_plugin():
    loader = importlib.machinery.SourceFileLoader("hermes_uteke_plugin", os.path.join(HERE, "__init__.py.tmpl"))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


class RecallParse(unittest.TestCase):
    def setUp(self):
        self.plugin = load_plugin()
        with open(FIXTURE) as f:
            self.cli_json = json.load(f)

    def test_real_cli_output_yields_a_hit(self):
        hits = self.plugin._parse_recall_results(self.cli_json)
        self.assertEqual(len(hits), 1)
        self.assertEqual(hits[0]["content"], "caching keeps hot data close to the reader")
        self.assertEqual(hits[0]["tags"], ["project:x"])

    def test_default_threshold_keeps_fusion_scores(self):
        # Default fusion scores are rank-based and top out near 0.04.
        # Hermetic: neither the env var nor a developer's $HERMES_HOME/uteke.json may set it.
        env = {k: v for k, v in os.environ.items() if k not in ("UTEKE_RECALL_MIN_SCORE", "HERMES_HOME")}
        old = os.environ.copy()
        os.environ.clear()
        os.environ.update(env)
        try:
            cfg = self.plugin._load_config()
        finally:
            os.environ.clear()
            os.environ.update(old)
        threshold = cfg["recall_min_score"]
        hits = [h for h in self.plugin._parse_recall_results(self.cli_json) if h["score"] >= threshold]
        self.assertEqual(len(hits), 1, f"default threshold {threshold} dropped the only hit")

    def test_nested_memory_shape_is_still_accepted(self):
        nested = [{"score": 0.5, "memory": {"content": "nested form", "tags": ["a"]}}]
        hits = self.plugin._parse_recall_results(nested)
        self.assertEqual([h["content"] for h in hits], ["nested form"])

    def test_items_without_content_are_dropped(self):
        self.assertEqual(self.plugin._parse_recall_results([{"score": 1.0}, "junk", {"content": ""}]), [])


if __name__ == "__main__":
    unittest.main()
