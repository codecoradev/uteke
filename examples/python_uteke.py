"""UtekeMemory — Python wrapper for the Uteke memory engine CLI.

Python wrapper for the uteke CLI binary. Wraps ``uteke`` via
subprocess calls with JSON output parsing. Covers **all** CLI commands
including namespace isolation, tag management, memory aging, and diagnostics.

Usage:
    from python_uteke import UtekeMemory

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
    """Error from the Uteke CLI.

    Attributes:
        returncode: Exit code of the CLI process (``-1`` if unavailable).
    """

    def __init__(self, message: str, returncode: int = -1) -> None:
        super().__init__(message)
        self.returncode = returncode


class UtekeMemory:
    """Python wrapper for Uteke memory engine — used by AI agents.

    All methods invoke the ``uteke`` CLI binary with ``--json`` and parse
    the structured output. The binary is resolved from ``$UTEKE_BIN`` env
    var or ``$PATH``.

    Args:
        store_path: Path to the Uteke store directory.
                    Defaults to ``~/.uteke``.
        namespace: Default namespace for multi-agent isolation.
                   Can be overridden per-method. Defaults to ``None``
                   (uses CLI default of ``"default"``).
    """

    def __init__(
        self,
        store_path: str = "~/.uteke",
        namespace: Optional[str] = None,
    ) -> None:
        self.store_path = os.path.expanduser(store_path)
        self._namespace = namespace
        self._uteke_bin = os.environ.get("UTEKE_BIN", "uteke")

    # ── Internal helpers ─────────────────────────────────────────────────

    def _ns_args(self, namespace: Optional[str]) -> List[str]:
        """Build namespace flag list.

        Args:
            namespace: Per-call namespace override. Falls back to the
                       instance-level ``_namespace`` if ``None``.

        Returns:
            ``["--namespace", name]`` or ``[]``.
        """
        ns = namespace if namespace is not None else self._namespace
        return ["--namespace", ns] if ns else []

    def _run(self, args: List[str]) -> str:
        """Run a uteke CLI command with ``--json`` and return stdout.

        Args:
            args: Command and flags to pass after the global options.

        Returns:
            Stripped stdout from the CLI process.

        Raises:
            UtekeError: If the CLI exits non-zero or times out (30 s).
        """
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
        """Parse JSON string, raising UtekeError on failure.

        Args:
            raw: JSON text returned by the CLI.

        Returns:
            Parsed Python object (dict, list, …).

        Raises:
            UtekeError: On invalid JSON.
        """
        try:
            return json.loads(raw)
        except json.JSONDecodeError as exc:
            raise UtekeError(f"Invalid JSON from uteke: {exc}") from exc

    # ── Core memories ────────────────────────────────────────────────────

    def remember(
        self,
        content: str,
        tags: Optional[List[str]] = None,
        namespace: Optional[str] = None,
    ) -> str:
        """Store a memory, return its ID.

        Args:
            content: The text to remember.
            tags: Optional list of tag strings.
            namespace: Namespace override.

        Returns:
            The UUID of the created memory.
        """
        args = self._ns_args(namespace) + ["remember", content]
        if tags:
            args.extend(["--tags", ",".join(tags)])
        data = self._parse_json(self._run(args))
        return data["id"]

    def recall(
        self,
        query: str,
        limit: int = 5,
        tags: Optional[List[str]] = None,
        namespace: Optional[str] = None,
    ) -> List[Dict[str, Any]]:
        """Semantic search for relevant memories.

        Args:
            query: Natural-language query.
            limit: Maximum results to return.
            tags: Optional tag filter.
            namespace: Namespace override.

        Returns:
            List of result dicts with ``memory`` and ``score`` keys.
        """
        args = self._ns_args(namespace) + [
            "recall", query, "--limit", str(limit),
        ]
        if tags:
            args.extend(["--tags", ",".join(tags)])
        return self._parse_json(self._run(args))

    def search(
        self,
        query: str,
        limit: int = 10,
        tags: Optional[List[str]] = None,
        namespace: Optional[str] = None,
    ) -> List[Dict[str, Any]]:
        """Full-text keyword search.

        Args:
            query: Keywords to search for.
            limit: Maximum results to return.
            tags: Optional tag filter.
            namespace: Namespace override.

        Returns:
            List of result dicts with ``memory`` and ``score`` keys.
        """
        args = self._ns_args(namespace) + [
            "search", query, "--limit", str(limit),
        ]
        if tags:
            args.extend(["--tags", ",".join(tags)])
        return self._parse_json(self._run(args))

    def list(
        self,
        tag: Optional[str] = None,
        limit: int = 20,
        offset: int = 0,
        namespace: Optional[str] = None,
    ) -> List[Dict[str, Any]]:
        """List memories with optional tag filter.

        Args:
            tag: Optional tag to filter by.
            limit: Maximum results.
            offset: Pagination offset.
            namespace: Namespace override.

        Returns:
            List of memory dicts.
        """
        args = self._ns_args(namespace) + [
            "list", "--limit", str(limit), "--offset", str(offset),
        ]
        if tag:
            args.extend(["--tag", tag])
        return self._parse_json(self._run(args))

    def get(
        self,
        memory_id: str,
        namespace: Optional[str] = None,
    ) -> Optional[Dict[str, Any]]:
        """Get a single memory by ID.

        Args:
            memory_id: UUID of the memory.
            namespace: Namespace override.

        Returns:
            Memory dict, or ``None`` if not found.
        """
        try:
            return self._parse_json(
                self._run(self._ns_args(namespace) + ["get", memory_id])
            )
        except UtekeError:
            return None

    def forget(
        self,
        memory_id: str,
        namespace: Optional[str] = None,
    ) -> bool:
        """Delete a memory.

        Args:
            memory_id: UUID of the memory to delete.
            namespace: Namespace override.

        Returns:
            ``True`` if deletion succeeded.
        """
        try:
            data = self._parse_json(
                self._run(
                    self._ns_args(namespace)
                    + ["forget", "--confirm", memory_id]
                )
            )
            return data.get("forgotten") == memory_id
        except UtekeError:
            return False

    def stats(
        self,
        namespace: Optional[str] = None,
    ) -> Dict[str, Any]:
        """Get store statistics.

        Args:
            namespace: Namespace override.

        Returns:
            Dict with ``total_memories``, ``unique_tags``, ``db_size_bytes``, etc.
        """
        return self._parse_json(
            self._run(self._ns_args(namespace) + ["stats"])
        )

    # ── Consolidate & Prune ──────────────────────────────────────────────

    def consolidate(
        self,
        threshold: float = 0.90,
        dry_run: bool = False,
        namespace: Optional[str] = None,
    ) -> List[Dict[str, Any]]:
        """Find and optionally merge near-duplicate memories.

        Args:
            threshold: Similarity threshold (0.0–1.0). Defaults to ``0.90``.
            dry_run: If ``True``, report candidates without merging.
            namespace: Namespace override.

        Returns:
            List of merge-candidate dicts when ``dry_run`` is set,
            or a summary dict otherwise.
        """
        args = self._ns_args(namespace) + [
            "consolidate", "--threshold", str(threshold),
        ]
        if dry_run:
            args.append("--dry-run")
        return self._parse_json(self._run(args))

    def prune(
        self,
        ttl_days: int = 30,
        dry_run: bool = False,
        namespace: Optional[str] = None,
    ) -> Dict[str, Any]:
        """Prune deprecated memories older than a TTL.

        Args:
            ttl_days: Age in days after which memories are deprecated.
                      Defaults to ``30``.
            dry_run: If ``True``, report candidates without deleting.
            namespace: Namespace override.

        Returns:
            Dict with ``pruned``, ``candidates``, etc.
        """
        args = self._ns_args(namespace) + ["prune", "--ttl", str(ttl_days)]
        if dry_run:
            args.append("--dry-run")
        return self._parse_json(self._run(args))

    # ── Namespace management ────────────────────────────────────────────

    def namespace_list(self) -> List[Dict[str, Any]]:
        """List all namespaces with memory counts.

        Returns:
            List of dicts each containing at least ``name`` and
            ``memory_count``.
        """
        return self._parse_json(self._run(["namespace", "list"]))

    def namespace_switch(self, name: str) -> None:
        """Set the default namespace in the Uteke config.

        Args:
            name: Namespace name to set as default.
        """
        self._run(["namespace", "switch", name])

    def namespace_stats(self, name: str) -> Dict[str, Any]:
        """Show statistics for a specific namespace.

        Args:
            name: Namespace name.

        Returns:
            Stats dict for the requested namespace.
        """
        return self._parse_json(self._run(["namespace", "stats", name]))

    # ── Tag management ──────────────────────────────────────────────────

    def tags_list(
        self,
        namespace: Optional[str] = None,
        by_count: bool = False,
    ) -> List[Dict[str, Any]]:
        """List all tags with usage counts.

        Args:
            namespace: Namespace override.
            by_count: Sort by count (descending) instead of alphabetical.

        Returns:
            List of tag dicts with ``tag`` and ``count`` keys.
        """
        args = self._ns_args(namespace) + ["tags", "list"]
        if by_count:
            args.append("--by-count")
        return self._parse_json(self._run(args))

    def tags_rename(
        self,
        old: str,
        new: str,
        namespace: Optional[str] = None,
    ) -> int:
        """Rename a tag across all memories.

        Args:
            old: Current tag name.
            new: New tag name.
            namespace: Namespace override.

        Returns:
            Number of memories updated.
        """
        data = self._parse_json(
            self._run(
                self._ns_args(namespace) + ["tags", "rename", old, new]
            )
        )
        return int(data.get("renamed", data.get("count", 0)))

    def tags_delete(
        self,
        tag: str,
        namespace: Optional[str] = None,
    ) -> int:
        """Delete a tag from all memories.

        Args:
            tag: Tag name to delete.
            namespace: Namespace override.

        Returns:
            Number of memories affected.
        """
        data = self._parse_json(
            self._run(
                self._ns_args(namespace)
                + ["tags", "delete", "--confirm", tag]
            )
        )
        return int(data.get("deleted", data.get("count", 0)))

    # ── Memory aging ─────────────────────────────────────────────────────

    def aging_status(
        self,
        namespace: Optional[str] = None,
    ) -> Dict[str, Any]:
        """Show aging status: hot, warm, cold, never-accessed counts.

        Args:
            namespace: Namespace override.

        Returns:
            Dict with aging tier counts.
        """
        return self._parse_json(
            self._run(self._ns_args(namespace) + ["aging", "status"])
        )

    def aging_preview(
        self,
        namespace: Optional[str] = None,
        days: int = 180,
    ) -> List[Dict[str, Any]]:
        """Preview memories eligible for cleanup (dry-run).

        Args:
            namespace: Namespace override.
            days: Minimum age in days. Defaults to ``180``.

        Returns:
            List of memory dicts that would be cleaned up.
        """
        return self._parse_json(
            self._run(
                self._ns_args(namespace)
                + ["aging", "preview", "--older-than-days", str(days)]
            )
        )

    def aging_cleanup(
        self,
        namespace: Optional[str] = None,
        days: int = 180,
    ) -> Dict[str, Any]:
        """Delete aged memories.

        Args:
            namespace: Namespace override.
            days: Minimum age in days. Defaults to ``180``.

        Returns:
            Dict with ``deleted`` count and details.
        """
        return self._parse_json(
            self._run(
                self._ns_args(namespace)
                + [
                    "aging", "cleanup",
                    "--older-than-days", str(days),
                    "--yes",
                ]
            )
        )

    # ── Diagnostics ─────────────────────────────────────────────────────

    def doctor(self) -> Dict[str, Any]:
        """Check system health (DB, index, model, consistency).

        Returns:
            Health-check dict with ``ok``, ``checks``, etc.
        """
        return self._parse_json(self._run(["doctor"]))

    def verify(
        self,
        namespace: Optional[str] = None,
    ) -> Dict[str, Any]:
        """Verify DB and index consistency.

        Args:
            namespace: Namespace override.

        Returns:
            Dict with ``consistent`` (bool) and optional ``issues`` list.
        """
        return self._parse_json(
            self._run(self._ns_args(namespace) + ["verify"])
        )

    def repair(
        self,
        namespace: Optional[str] = None,
    ) -> Dict[str, Any]:
        """Repair index by rebuilding from SQLite.

        Args:
            namespace: Namespace override.

        Returns:
            Dict with ``repaired`` count and details.
        """
        return self._parse_json(
            self._run(self._ns_args(namespace) + ["repair"])
        )

    # ── Import / Export ─────────────────────────────────────────────────

    def export(
        self,
        path: str,
        namespace: Optional[str] = None,
    ) -> None:
        """Export all memories to a JSONL file.

        Writes portable JSONL (no embeddings) to *path*.

        Args:
            path: Destination file path.
            namespace: Namespace override.
        """
        self._run(self._ns_args(namespace) + ["export", path])

    def import_from(
        self,
        path: str,
        namespace: Optional[str] = None,
    ) -> Dict[str, Any]:
        """Import memories from a JSONL file (re-embeds content).

        Args:
            path: Source JSONL file path.
            namespace: Namespace override.

        Returns:
            Dict with ``imported`` count and details.
        """
        return self._parse_json(
            self._run(self._ns_args(namespace) + ["import", path])
        )


# ── Standalone smoke test ───────────────────────────────────────────────

if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="uteke_smoke_") as tmpdir:
        mem = UtekeMemory(store_path=tmpdir)
        print(f"Store: {tmpdir}")

        # remember
        mid = mem.remember(
            "Hello from Python wrapper", tags=["test", "smoke"]
        )
        print(f"Remembered: {mid}")

        # recall
        results = mem.recall("hello python")
        print(f"Recall results: {len(results)}")

        # search (with tags)
        results = mem.search("Python", tags=["test"])
        print(f"Search results (tag=test): {len(results)}")

        # list
        items = mem.list(tag="smoke")
        print(f"List (tag=smoke): {len(items)}")

        # get
        got = mem.get(mid)
        print(f"Get: {got['content'] if got else 'NOT FOUND'}")

        # stats
        s = mem.stats()
        print(f"Stats: {s['total_memories']} memories")

        # tags_list
        tags = mem.tags_list()
        print(f"Tags: {[t.get('tag', t) for t in tags]}")

        # doctor
        doc = mem.doctor()
        statuses = {c["status"] for c in doc.get("checks", [])}
        print(f"Doctor: all ok = {statuses == {'Ok'}}")

        # forget
        ok = mem.forget(mid)
        print(f"Forget: {ok}")
        assert ok, "forget should return True"

        # stats after forget
        s = mem.stats()
        assert s["total_memories"] == 0, (
            f"Expected 0 memories after forget, got {s['total_memories']}"
        )
        print(f"Stats after forget: {s['total_memories']} memories")

        # namespace support — remember in an isolated namespace
        mem_ns = UtekeMemory(store_path=tmpdir, namespace="smoke-ns")
        mid_ns = mem_ns.remember("Namespace isolated memory", tags=["ns"])
        print(f"Remembered in namespace: {mid_ns}")

        results_ns = mem_ns.recall("namespace")
        print(f"Recall in namespace: {len(results_ns)}")

        stats_ns = mem_ns.stats()
        print(f"Namespace stats: {stats_ns['total_memories']} memories")

        ok_ns = mem_ns.forget(mid_ns)
        print(f"Forget in namespace: {ok_ns}")
        assert ok_ns, "forget in namespace should return True"

        print("✓ Smoke test passed")
