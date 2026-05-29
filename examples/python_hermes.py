"""UtekeMemory — Python wrapper for the Uteke memory engine CLI.

Designed for use with Hermes AI agents. Wraps the `uteke` binary via
subprocess calls with JSON output parsing.

Usage:
    from python_hermes import UtekeMemory

    mem = UtekeMemory()
    mid = mem.remember("Deploy v2.1 to staging", tags=["deploy", "staging"])
    results = mem.recall("deployment steps")
    mem.forget(mid)

No external dependencies — stdlib only. Requires Python 3.8+.
"""

import json
import os
import subprocess
import tempfile
from typing import Any, Dict, List, Optional


class UtekeError(Exception):
    """Error from the Uteke CLI."""

    def __init__(self, message: str, returncode: int = -1) -> None:
        super().__init__(message)
        self.returncode = returncode


class UtekeMemory:
    """Python wrapper for Uteke memory engine — used by Hermes AI agents.

    All methods invoke the ``uteke`` CLI binary with ``--json`` and parse
    the structured output. The binary is resolved from ``$UTEKE_BIN`` env
    var or ``$PATH``.

    Args:
        store_path: Path to the Uteke store directory.
                    Defaults to ``~/.uteke``.
    """

    def __init__(self, store_path: str = "~/.uteke") -> None:
        self.store_path = os.path.expanduser(store_path)
        self._uteke_bin = os.environ.get("UTEKE_BIN", "uteke")

    # ── Internal helpers ─────────────────────────────────────────────────

    def _run(self, args: List[str]) -> str:
        """Run a uteke CLI command with ``--json`` and return stdout."""
        cmd = [self._uteke_bin, "--json", "--store", self.store_path] + args
        try:
            result = subprocess.run(
                cmd,
                capture_output=True,
                text=True,
                check=True,
                timeout=30,
            )
        except subprocess.CalledProcessError as exc:
            stderr = exc.stderr.strip() if exc.stderr else ""
            raise UtekeError(
                f"uteke {' '.join(args)} failed: {stderr}",
                returncode=exc.returncode,
            ) from exc
        except subprocess.TimeoutExpired as exc:
            raise UtekeError(
                f"uteke {' '.join(args)} timed out after 30s"
            ) from exc
        return result.stdout.strip()

    @staticmethod
    def _parse_json(raw: str) -> Any:
        """Parse JSON string, raising UtekeError on failure."""
        try:
            return json.loads(raw)
        except json.JSONDecodeError as exc:
            raise UtekeError(f"Invalid JSON from uteke: {exc}") from exc

    # ── Public API ───────────────────────────────────────────────────────

    def remember(self, content: str, tags: Optional[List[str]] = None) -> str:
        """Store a memory, return its ID.

        Args:
            content: The text to remember.
            tags: Optional list of tag strings.

        Returns:
            The UUID of the created memory.
        """
        args = ["remember", content]
        if tags:
            args.extend(["--tags", ",".join(tags)])
        data = self._parse_json(self._run(args))
        return data["id"]

    def recall(
        self,
        query: str,
        limit: int = 5,
        tags: Optional[List[str]] = None,
    ) -> List[Dict[str, Any]]:
        """Semantic search for relevant memories.

        Args:
            query: Natural-language query.
            limit: Maximum results to return.
            tags: Optional tag filter.

        Returns:
            List of result dicts with ``memory`` and ``score`` keys.
        """
        args = ["recall", query, "--limit", str(limit)]
        if tags:
            args.extend(["--tags", ",".join(tags)])
        return self._parse_json(self._run(args))

    def search(self, query: str, limit: int = 10) -> List[Dict[str, Any]]:
        """Full-text keyword search.

        Args:
            query: Keywords to search for.
            limit: Maximum results to return.

        Returns:
            List of result dicts with ``memory`` and ``score`` keys.
        """
        args = ["search", query, "--limit", str(limit)]
        return self._parse_json(self._run(args))

    def list(
        self,
        tag: Optional[str] = None,
        limit: int = 20,
        offset: int = 0,
    ) -> List[Dict[str, Any]]:
        """List memories with optional tag filter.

        Args:
            tag: Optional tag to filter by.
            limit: Maximum results.
            offset: Pagination offset.

        Returns:
            List of memory dicts.
        """
        args = ["list", "--limit", str(limit), "--offset", str(offset)]
        if tag:
            args.extend(["--tag", tag])
        return self._parse_json(self._run(args))

    def get(self, memory_id: str) -> Optional[Dict[str, Any]]:
        """Get a single memory by ID.

        Args:
            memory_id: UUID of the memory.

        Returns:
            Memory dict, or ``None`` if not found.
        """
        try:
            return self._parse_json(self._run(["get", memory_id]))
        except UtekeError:
            return None

    def forget(self, memory_id: str) -> bool:
        """Delete a memory.

        Args:
            memory_id: UUID of the memory to delete.

        Returns:
            ``True`` if deletion succeeded.
        """
        try:
            data = self._parse_json(self._run(["forget", memory_id]))
            return data.get("forgotten") == memory_id
        except UtekeError:
            return False

    def stats(self) -> Dict[str, Any]:
        """Get store statistics.

        Returns:
            Dict with ``total_memories``, ``unique_tags``, ``db_size_bytes``.
        """
        return self._parse_json(self._run(["stats"]))


# ── Standalone smoke test ─────────────────────────────────────────────────

if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="uteke_smoke_") as tmpdir:
        mem = UtekeMemory(store_path=tmpdir)
        print(f"Store: {tmpdir}")

        mid = mem.remember("Hello from Python wrapper", tags=["test", "smoke"])
        print(f"Remembered: {mid}")

        results = mem.recall("hello python")
        print(f"Recall results: {len(results)}")

        results = mem.search("Python")
        print(f"Search results: {len(results)}")

        items = mem.list(tag="smoke")
        print(f"List (tag=smoke): {len(items)}")

        got = mem.get(mid)
        print(f"Get: {got['content'] if got else 'NOT FOUND'}")

        s = mem.stats()
        print(f"Stats: {s['total_memories']} memories")

        ok = mem.forget(mid)
        print(f"Forget: {ok}")

        s = mem.stats()
        print(f"Stats after forget: {s['total_memories']} memories")
        print("✓ Smoke test passed")
