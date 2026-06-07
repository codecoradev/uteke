"""Integration tests for the UtekeMemory Python wrapper.

Creates a temporary store, exercises all wrapper methods, and cleans up.
Run with: python examples/test_uteke_integration.py

Expects the ``uteke`` binary to be available on ``$PATH`` (or set
``$UTEKE_BIN`` to its location).
"""

import os
import sys
import tempfile
import unittest

# Allow importing from examples/ when run from project root
sys.path.insert(0, os.path.join(os.path.dirname(__file__)))

from python_uteke import UtekeError, UtekeMemory  # noqa: E402


class TestUtekeMemoryIntegration(unittest.TestCase):
    """End-to-end integration tests for UtekeMemory."""

    @classmethod
    def setUpClass(cls) -> None:
        """Verify uteke binary is available."""
        cls.uteke_bin = os.environ.get("UTEKE_BIN", "uteke")
        try:
            UtekeMemory(store_path="/tmp/uteke_integration_probe")
        except UtekeError:
            raise unittest.SkipTest(
                f"uteke binary not found (tried: {cls.uteke_bin}). "
                "Set UTEKE_BIN env var or add uteke to PATH."
            )

    def setUp(self) -> None:
        """Create a fresh temporary store for each test."""
        self._tmpdir = tempfile.mkdtemp(prefix="uteke_test_")
        self.mem = UtekeMemory(store_path=self._tmpdir)

    def tearDown(self) -> None:
        """Remove temporary store."""
        import shutil
        shutil.rmtree(self._tmpdir, ignore_errors=True)

    # ── Tests ────────────────────────────────────────────────────────────

    def test_remember_returns_id(self) -> None:
        """remember() returns a non-empty UUID string."""
        mid = self.mem.remember("Integration test memory")
        self.assertIsInstance(mid, str)
        self.assertTrue(len(mid) > 0)

    def test_remember_with_tags(self) -> None:
        """remember() stores tags correctly."""
        mid = self.mem.remember(
            "Tagged memory", tags=["alpha", "beta"]
        )
        got = self.mem.get(mid)
        assert got is not None
        self.assertIn("alpha", got["tags"])
        self.assertIn("beta", got["tags"])

    def test_recall_finds_relevant(self) -> None:
        """recall() returns semantically relevant results."""
        self.mem.remember("The cat sat on the mat", tags=["animals"])
        self.mem.remember("Python is a programming language", tags=["tech"])
        self.mem.remember("Dogs are loyal companions", tags=["animals"])

        results = self.mem.recall("feline pet", limit=3)
        self.assertGreaterEqual(len(results), 1)
        # At least one animal-related result should appear
        contents = " ".join(r["memory"]["content"].lower() for r in results)
        self.assertTrue(
            "cat" in contents or "dog" in contents,
            f"Expected animal content in results, got: {contents}",
        )

    def test_recall_with_tag_filter(self) -> None:
        """recall() with tag filter excludes non-matching memories."""
        self.mem.remember("Deploy to production", tags=["deploy"])
        self.mem.remember("Feed the cat", tags=["pets"])

        results = self.mem.recall("deploy", limit=5, tags=["deploy"])
        for r in results:
            self.assertIn("deploy", r["memory"]["tags"])

    def test_search_keyword_match(self) -> None:
        """search() finds keyword matches."""
        self.mem.remember("Kubernetes pod crash loop backoff")
        self.mem.remember("The weather is sunny today")

        results = self.mem.search("Kubernetes")
        self.assertGreaterEqual(len(results), 1)
        self.assertIn(
            "Kubernetes", results[0]["memory"]["content"]
        )

    def test_list_all(self) -> None:
        """list() returns stored memories."""
        for i in range(5):
            self.mem.remember(f"Memory item {i}")
        items = self.mem.list(limit=10)
        self.assertGreaterEqual(len(items), 5)

    def test_list_with_tag_filter(self) -> None:
        """list() filters by tag."""
        self.mem.remember("Tagged A", tags=["tag-a"])
        self.mem.remember("Tagged B", tags=["tag-b"])
        self.mem.remember("No tag here")

        items = self.mem.list(tag="tag-a")
        self.assertEqual(len(items), 1)
        self.assertEqual(items[0]["content"], "Tagged A")

    def test_list_pagination(self) -> None:
        """list() respects limit and offset."""
        for i in range(6):
            self.mem.remember(f"Page item {i}")

        page1 = self.mem.list(limit=3, offset=0)
        page2 = self.mem.list(limit=3, offset=3)
        self.assertEqual(len(page1), 3)
        self.assertEqual(len(page2), 3)
        # Pages should not overlap
        ids1 = {m["id"] for m in page1}
        ids2 = {m["id"] for m in page2}
        self.assertEqual(len(ids1 & ids2), 0)

    def test_get_existing(self) -> None:
        """get() returns the memory dict for a valid ID."""
        mid = self.mem.remember("Fetch me")
        got = self.mem.get(mid)
        assert got is not None
        self.assertEqual(got["id"], mid)
        self.assertEqual(got["content"], "Fetch me")

    def test_get_nonexistent(self) -> None:
        """get() returns None for a missing ID."""
        got = self.mem.get("00000000-0000-0000-0000-000000000000")
        self.assertIsNone(got)

    def test_forget_deletes(self) -> None:
        """forget() removes the memory."""
        mid = self.mem.remember("Delete me")
        self.assertTrue(self.mem.forget(mid))
        self.assertIsNone(self.mem.get(mid))

    def test_forget_nonexistent(self) -> None:
        """forget() for a missing ID — CLI still returns success."""
        # The CLI returns {"forgotten": id} even for non-existent IDs
        # since the delete is idempotent from the CLI perspective.
        # We verify it doesn't raise an error.
        result = self.mem.forget("00000000-0000-0000-0000-000000000000")
        # CLI returns True because it exits 0 and returns the forgotten key
        self.assertIsInstance(result, bool)

    def test_stats_counts(self) -> None:
        """stats() returns correct counts."""
        self.mem.remember("S1", tags=["a", "b"])
        self.mem.remember("S2", tags=["a"])

        s = self.mem.stats()
        self.assertEqual(s["total_memories"], 2)
        self.assertGreaterEqual(s["unique_tags"], 2)
        self.assertGreater(s["db_size_bytes"], 0)

    def test_stats_after_forget(self) -> None:
        """stats() reflects deletion."""
        mid = self.mem.remember("Temporary")
        s = self.mem.stats()
        self.assertEqual(s["total_memories"], 1)

        self.mem.forget(mid)
        s = self.mem.stats()
        self.assertEqual(s["total_memories"], 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
