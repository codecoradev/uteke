"""Static checks that integrations/agent-plugin satisfies the Agent Plugins v1 rules
Hermes enforces (hermes_cli/agent_plugins.py), so a bad edit fails CI here."""
import json
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2] / "integrations" / "agent-plugin"
PLUGIN_SCHEMA = "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json"
MCP_SCHEMA = "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json"
PLUGIN_FIELDS = {"$schema", "name", "version", "description", "author", "homepage",
                 "repository", "license", "keywords", "extensions"}
NAME_RE = re.compile(r"^(?!.*(?:--|\.\.))[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?$")
SKILL_RE = re.compile(r"^(?!.*--)[a-z0-9]+(?:-[a-z0-9]+)*$")


def frontmatter(path):
    text = path.read_text(encoding="utf-8")
    assert text.startswith("---"), "missing frontmatter"
    end = re.search(r"\n---\s*\n", text[3:])
    assert end, "unterminated frontmatter"
    out = {}
    for line in text[3:end.start() + 3].splitlines():
        if ":" in line and not line.startswith(" "):
            k, v = line.split(":", 1)
            out[k.strip()] = v.strip()
    return out


class AgentPlugin(unittest.TestCase):
    def test_manifest(self):
        m = json.loads((ROOT / "plugin.json").read_text())
        self.assertEqual(set(m) - PLUGIN_FIELDS, set())
        self.assertEqual(m["$schema"], PLUGIN_SCHEMA)
        self.assertTrue(1 <= len(m["name"]) <= 64 and NAME_RE.fullmatch(m["name"]))
        for k in ("version", "description", "homepage", "repository", "license"):
            self.assertIsInstance(m[k], str)
        self.assertEqual(set(m["author"]) - {"name", "email", "url"}, set())

    def test_skills(self):
        dirs = [p for p in (ROOT / "skills").iterdir() if p.is_dir()]
        self.assertTrue(dirs)
        for d in dirs:
            fm = frontmatter(d / "SKILL.md")
            self.assertEqual(fm["name"], d.name)
            self.assertTrue(SKILL_RE.fullmatch(d.name))
            self.assertTrue(1 <= len(fm["description"]) <= 1024)

    def test_mcp(self):
        c = json.loads((ROOT / "mcp.json").read_text())
        self.assertEqual(set(c), {"$schema", "mcpServers"})
        self.assertEqual(c["$schema"], MCP_SCHEMA)
        for name, s in c["mcpServers"].items():
            self.assertEqual(s["type"], "stdio", name)
            self.assertEqual(set(s) - {"type", "command", "args", "env", "cwd"}, set())
            cmd = s["command"]
            self.assertTrue(cmd.startswith("./") or not re.search(r"[/\\\s]", cmd), cmd)
            self.assertNotIn("env", s, "no credentials or env in a catalog plugin")

    def test_no_binaries_or_symlinks(self):
        for p in ROOT.rglob("*"):
            self.assertFalse(p.is_symlink(), p)
            if p.is_file():
                self.assertNotIn(p.suffix, {".exe", ".so", ".dylib", ".dll", ".bin"}, p)


if __name__ == "__main__":
    unittest.main()
