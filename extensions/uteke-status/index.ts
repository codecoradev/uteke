/**
 * Uteke Memory Status Extension
 *
 * Shows uteke memory stats in the pi footer status bar.
 * Displays: 🧠 uteke:  🔥 4 hot  🟡 0 warm  ❄️ 63 cold  (67 total)
 *
 * Also injects a system prompt reminder to use uteke for memory.
 * Queries SQLite directly for cross-namespace totals.
 *
 * Install: ~/.pi/agent/extensions/uteke-status/index.ts
 * Project-local: .pi/extensions/uteke-status/index.ts
 */

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { execSync } from "node:child_process";
import { join } from "node:path";
import { homedir } from "node:os";

interface Stats {
	total: number;
	hot: number;
	warm: number;
	cold: number;
}

function getAllStats(): Stats | null {
	const dbPath = join(homedir(), ".uteke", "uteke.db");

	const sql = [
		"SELECT COUNT(*) as total,",
		"SUM(CASE WHEN last_accessed >= datetime('now','-7 days') THEN 1 ELSE 0 END) as hot,",
		"SUM(CASE WHEN last_accessed >= datetime('now','-30 days') AND last_accessed < datetime('now','-7 days') THEN 1 ELSE 0 END) as warm",
		"FROM memories;",
	].join(" ");

	try {
		const out = execSync(`sqlite3 '${dbPath}' "${sql}"`, {
			timeout: 5000,
			encoding: "utf-8",
		}).trim();

		const parts = out.split("|");
		if (parts.length < 3) return null;

		const total = parseInt(parts[0], 10);
		const hot = parseInt(parts[1], 10);
		const warm = parseInt(parts[2], 10);
		const cold = total - hot - warm;

		return { total, hot, warm, cold };
	} catch {
		return null;
	}
}

export default function (pi: ExtensionAPI) {
	let available = false;

	function updateStatus(ctx: any) {
		const stats = getAllStats();
		if (!stats) {
			ctx.ui.setStatus("uteke", "🧠 uteke: not found");
			return;
		}
		available = true;
		ctx.ui.setStatus(
			"uteke",
			`🧠 uteke:   🔥 ${stats.hot} hot   🟡 ${stats.warm} warm   ❄️ ${stats.cold} cold   (${stats.total} total)`
		);
	}

	// Inject system prompt to remind agent to use uteke
	pi.on("before_agent_start", async (event, ctx) => {
		if (!available) return;

		return {
			systemPrompt:
				event.systemPrompt +
				"\n\n## uteke Memory\n" +
				"You have access to uteke — a local persistent memory engine. Use it actively:\n" +
				"- `uteke remember` — Save important context, decisions, progress, architecture notes\n" +
				"- `uteke recall` — Search memories by meaning before asking the user\n" +
				"- `uteke search` — Keyword search across memories\n" +
				"- `uteke stats` — Check memory store status\n" +
				"- `uteke remember --tags <tags> --namespace <ns>` — Organize by tags and namespace\n" +
				"\n" +
				"Rules:\n" +
				"1. Save important context to uteke proactively (decisions, progress, architecture)\n" +
				"2. Before starting a task, recall relevant memories to restore context\n" +
				"3. Use namespaces to isolate memory per project or agent role\n" +
				"4. Tag memories with descriptive tags for easy filtering\n" +
				"5. When ending a session, save a summary of what was done\n",
		};
	});

	pi.on("session_start", async (_event, ctx) => {
		updateStatus(ctx);
	});

	pi.on("turn_end", async (_event, ctx) => {
		if (!available) return;
		updateStatus(ctx);
	});

	pi.registerCommand("uteke-stats", {
		description: "Refresh uteke memory stats in status bar",
		handler: async (_args, ctx) => {
			updateStatus(ctx);
		},
	});
}
