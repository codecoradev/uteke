/**
 * Uteke Memory Provider Extension for Pi
 *
 * Automatic persistent memory integration — mirrors the Hermes memory-provider
 * experience. Hooks `before_agent_start` to inject relevant memories into
 * every agent turn without manual tool calls.
 *
 * Install:
 *   Global:  ~/.pi/agent/extensions/uteke-memory-provider/index.ts
 *   Project: .pi/extensions/uteke-memory-provider/index.ts
 *
 * Requires: `uteke` binary on PATH (same binary as the CLI).
 */

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { execSync } from "node:child_process";
import { join } from "node:path";
import { homedir } from "node:os";

// ---------------------------------------------------------------------------
// Config (reads from env, can be extended)
// ---------------------------------------------------------------------------

const DEFAULT_NAMESPACE = "default";
const RECALL_LIMIT = 6;
const RECALL_MIN_SCORE = 0.45;
const RECALL_TIMEOUT_MS = 15_000;

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

function recallMemories(query: string): string[] {
	const ns = process.env.UTEKE_NAMESPACE || DEFAULT_NAMESPACE;
	const limit = process.env.UTEKE_RECALL_LIMIT || String(RECALL_LIMIT);
	const minScore = process.env.UTEKE_RECALL_MIN_SCORE || String(RECALL_MIN_SCORE);

	const raw = runUteke([
		"recall", `"${query}"`,
		"--namespace", ns,
		"--limit", limit,
		"--min-score", minScore,
		"--json",
	]);

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

function formatMemories(memories: string[]): string {
	if (!memories.length) return "";
	const lines = memories.map((m, i) => `${i + 1}. ${m}`).join("\n");
	return (
		"\n\n## Relevant Memories (uteke)\n\n" +
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

		if (available) {
			ctx.ui.setStatus("uteke", "🧠 uteke: active");
		} else {
			ctx.ui.setStatus("uteke", "🧠 uteke: not found");
		}
	});

	// Core hook: inject relevant memories before every agent turn
	pi.on("before_agent_start", async (event) => {
		if (!available) return;

		const prompt = (event.prompt || "").trim();
		if (!prompt || prompt.length < 3) return;

		const memories = recallMemories(prompt);
		if (!memories.length) return;

		const injection = formatMemories(memories);

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
			const memories = recallMemories(query);
			if (!memories.length) return "No relevant memories found.";
			return formatMemories(memories);
		},
	});

	pi.registerCommand("uteke-save", {
		description: "Save a memory to uteke",
		handler: async (args, _ctx) => {
			const content = args.join(" ").trim();
			if (!content) return "Usage: /uteke-save <content>";
			runUteke(["remember", `"${content}"`], RECALL_TIMEOUT_MS);
			return "✓ Memory saved.";
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
