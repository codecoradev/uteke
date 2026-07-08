/**
 * Uteke Memory Provider Extension for Pi
 *
 * Automatic persistent memory integration — mirrors the Hermes memory-provider
 * experience. Hooks `before_agent_start` to inject relevant memories into
 * every agent turn without manual tool calls.
 *
 * Project-aware: auto-detects project name from the current working directory
 * and tags all memories with `project:<name>` for noise-free recall.
 *
 * Install:
 *   Global:  ~/.pi/agent/extensions/uteke-memory-provider/index.ts
 *   Project: .pi/extensions/uteke-memory-provider/index.ts
 *
 * Requires: `uteke` binary on PATH (same binary as the CLI).
 */

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { execSync } from "node:child_process";
import { basename, resolve } from "node:path";
import { existsSync } from "node:fs";

// ---------------------------------------------------------------------------
// Config (reads from env, can be extended)
// ---------------------------------------------------------------------------

const DEFAULT_NAMESPACE = "default";
const RECALL_LIMIT = 6;
const RECALL_MIN_SCORE = 0.45;
const RECALL_TIMEOUT_MS = 15_000;

// Known project root directories to scan for project detection
const KNOWN_PROJECT_DIRS = [
	"/opt/data/repos",
	"/home",
	process.env.HOME || "",
].filter(Boolean);

// ---------------------------------------------------------------------------
// Project Detection
// ---------------------------------------------------------------------------

/**
 * Detect project name from the current working directory.
 * Walks up from cwd to find a known project root, then extracts the folder name.
 * Returns lowercase project name or empty string.
 */
function detectProjectFromCwd(): string {
	const cwd = process.cwd();

	for (const root of KNOWN_PROJECT_DIRS) {
		if (!root || !existsSync(root)) continue;
		const resolved = resolve(root);

		// Check if cwd is inside or equal to this project root
		if (cwd.startsWith(resolved + "/") || cwd === resolved) {
			// Extract the immediate subdirectory name
			const relative = cwd.slice(resolved.length + 1);
			const firstSegment = relative.split("/")[0];
			if (firstSegment && !firstSegment.startsWith(".")) {
				return firstSegment.toLowerCase();
			}
		}
	}

	return "";
}

/**
 * Detect project name from text (file paths, mentions).
 * Falls back to cwd detection if nothing found in text.
 */
function detectProject(text: string, fallbackToCwd = false): string {
	if (!text) return fallbackToCwd ? detectProjectFromCwd() : "";

	// Strategy 1: file paths containing known project roots
	for (const root of KNOWN_PROJECT_DIRS) {
		if (!root) continue;
		const escaped = root.replace(/\//g, "\\/");
		const match = text.match(new RegExp(`${escaped}/([a-zA-Z0-9_.-]+)`));
		if (match && match[1] && !match[1].startsWith(".")) {
			return match[1].toLowerCase();
		}
	}

	return fallbackToCwd ? detectProjectFromCwd() : "";
}

// ---------------------------------------------------------------------------
// Subprocess helpers
// ---------------------------------------------------------------------------

function utekeBin(): string {
	const envBin = process.env.UTEKE_BIN;
	if (envBin) return envBin;
	// pi extensions run as Node.js — use which-compatible lookup
	try {
		return execSync("which uteke", { encoding: "utf-8", timeout: 3000 }).trim();
	} catch {
		return "uteke";
	}
}

function runUteke(args: string[], timeout = RECALL_TIMEOUT_MS): string {
	const bin = utekeBin();
	const env = { ...process.env };
	// Allow pointing uteke at a different home directory
	if (process.env.UTEKE_HOME) {
		env.HOME = process.env.UTEKE_HOME;
	}
	try {
		return execSync(`${bin} ${args.join(" ")}`, {
			encoding: "utf-8",
			timeout,
			env,
		});
	} catch (err: any) {
		return "";
	}
}

function recallMemories(query: string, project: string): string[] {
	const ns = process.env.UTEKE_NAMESPACE || DEFAULT_NAMESPACE;
	const limit = process.env.UTEKE_RECALL_LIMIT || String(RECALL_LIMIT);
	const minScore = process.env.UTEKE_RECALL_MIN_SCORE || String(RECALL_MIN_SCORE);

	const args: string[] = [
		"recall", `"${query}"`,
		"--namespace", ns,
		"--limit", limit,
		"--min-score", minScore,
		"--json",
	];

	// Filter by project tag if detected
	if (project) {
		args.push("--tags", `project:${project}`);
	}

	const raw = runUteke(args);

	if (!raw.trim()) return [];

	try {
		const items = JSON.parse(raw);
		const results: string[] = [];
		for (const item of items) {
			const mem = item.memory || item;
			const content = mem.content || "";
			if (content) {
				results.push(content);
			}
		}
		return results;
	} catch {
		return [];
	}
}

function formatMemories(memories: string[], project: string): string {
	if (!memories.length) return "";
	const projectHint = project ? ` [project: ${project}]` : "";
	const lines = memories.map((m, i) => `${i + 1}. ${m}`).join("\n");
	return (
		`\n\n## Relevant Memories (uteke)${projectHint}\n\n` +
		lines +
		"\n\nUse `uteke remember` to store new facts, `uteke recall` to search more."
	);
}

// ---------------------------------------------------------------------------
// Extension
// ---------------------------------------------------------------------------

export default function (pi: ExtensionAPI) {
	let available = false;

	// Check uteke availability on session start
	pi.on("session_start", async (_event, ctx) => {
		const out = runUteke(["stats"], 3000);
		available = out.trim().length > 0;

		const project = detectProjectFromCwd();
		const status = project
			? `🧠 uteke: active (${project})`
			: available
				? "🧠 uteke: active"
				: "🧠 uteke: not found";

		ctx.ui.setStatus("uteke", status);
	});

	// Core hook: inject relevant memories before every agent turn
	pi.on("before_agent_start", async (event) => {
		if (!available) return;

		const prompt = (event.prompt || "").trim();
		if (!prompt || prompt.length < 3) return;

		// Detect project from prompt text, fallback to cwd
		const project = detectProject(prompt, true);
		const memories = recallMemories(prompt, project);
		if (!memories.length) return;

		const injection = formatMemories(memories, project);

		return {
			systemPrompt: event.systemPrompt + injection,
		};
	});

	// Slash commands for manual control
	pi.registerCommand("uteke-recall", {
		description: "Manually recall memories for a topic",
		handler: async (args, _ctx) => {
			const query = args.join(" ").trim();
			if (!query) return "Usage: /uteke-recall <topic>";
			const project = detectProject(query, true);
			const memories = recallMemories(query, project);
			if (!memories.length) return "No relevant memories found.";
			return formatMemories(memories, project);
		},
	});

	pi.registerCommand("uteke-save", {
		description: "Save a memory to uteke (auto-tagged with project)",
		handler: async (args, _ctx) => {
			const content = args.join(" ").trim();
			if (!content) return "Usage: /uteke-save <content>";

			const ns = process.env.UTEKE_NAMESPACE || DEFAULT_NAMESPACE;
			const project = detectProjectFromCwd();
			const cmdArgs: string[] = [
				"remember", `"${content}"`,
				"--namespace", ns,
			];
			if (project) {
				cmdArgs.push("--tags", `project:${project}`);
			}

			runUteke(cmdArgs, RECALL_TIMEOUT_MS);
			const projectNote = project ? ` [project: ${project}]` : "";
			return `✓ Memory saved.${projectNote}`;
		},
	});

	pi.registerCommand("uteke-stats", {
		description: "Show uteke memory stats",
		handler: async (_args, ctx) => {
			const out = runUteke(["stats"], 5000);
			available = out.trim().length > 0;
			ctx.ui.setStatus("uteke", available ? "🧠 uteke: active" : "🧠 uteke: not found");
			return out.trim() || "uteke: not available";
		},
	});

	pi.registerCommand("uteke-on", {
		description: "Enable automatic memory recall",
		handler: async () => {
			available = true;
			return "✓ Automatic uteke recall enabled.";
		},
	});

	pi.registerCommand("uteke-off", {
		description: "Disable automatic memory recall",
		handler: async () => {
			available = false;
			return "✓ Automatic uteke recall disabled.";
		},
	});
}
