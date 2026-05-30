/**
 * Uteke Memory Status Extension
 *
 * Shows uteke memory stats in the pi footer status bar.
 * Displays: 🧠 uteke: 3 hot | 2 warm | 4 cold (9 total)
 * 
 * Updates on session start and after memory-related tool calls.
 * 
 * Install: Place in ~/.pi/agent/extensions/uteke-status/index.ts
 * Or project-local: .pi/extensions/uteke-status/index.ts
 */

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { execFile } from "node:child_process";

interface UtekeStats {
	total_memories: number;
	unique_tags: number;
	db_size_bytes: number;
	hot: number;
	warm: number;
	cold: number;
}

function getStats(namespace?: string): Promise<UtekeStats | null> {
	return new Promise((resolve) => {
		const args = ["stats", "--json"];
		if (namespace) args.push("--namespace", namespace);

		execFile("uteke", args, { timeout: 10000 }, (error, stdout) => {
			if (error) {
				resolve(null);
				return;
			}
			try {
				resolve(JSON.parse(stdout));
			} catch {
				resolve(null);
			}
		});
	});
}

function formatStats(stats: UtekeStats, theme: any): string {
	const hot = theme.fg("red", `🔥${stats.hot}`);
	const warm = theme.fg("yellow", `🟡${stats.warm}`);
	const cold = theme.fg("blue", `❄️${stats.cold}`);
	const total = theme.fg("dim", `(${stats.total_memories} total)`);
	return `${hot} ${warm} ${cold} ${total}`;
}

export default function (pi: ExtensionAPI) {
	// Track if uteke is available
	let utekeAvailable = false;
	let lastStats: UtekeStats | null = null;

	async function refreshStatus(ctx: any) {
		if (!utekeAvailable) return;

		const stats = await getStats();
		if (!stats) return;

		lastStats = stats;
		const theme = ctx.ui.theme;
		const brain = theme.fg("accent", "🧠 uteke:");
		const statusText = `${brain} ${formatStats(stats, theme)}`;
		ctx.ui.setStatus("uteke", statusText);
	}

	pi.on("session_start", async (_event, ctx) => {
		// Check if uteke is available
		const stats = await getStats();
		if (stats) {
			utekeAvailable = true;
			lastStats = stats;
			const theme = ctx.ui.theme;
			const brain = theme.fg("accent", "🧠 uteke:");
			ctx.ui.setStatus("uteke", `${brain} ${formatStats(stats, theme)}`);
		} else {
			utekeAvailable = false;
			const theme = ctx.ui.theme;
			ctx.ui.setStatus("uteke", theme.fg("dim", "🧠 uteke: not installed"));
		}
	});

	// Refresh after memory-related commands (remember, recall, forget, import, cleanup)
	pi.on("tool_call", async (event, _ctx) => {
		if (!utekeAvailable) return;
		
		const memoryCommands = ["bash"];
		if (!memoryCommands.includes(event.toolName)) return;

		const cmd = (event as any).input?.command || "";
		const isMemoryOp = /\buteke\s+(remember|recall|forget|import|aging\s+cleanup)\b/.test(cmd);
		if (!isMemoryOp) return;
	});

	// Refresh stats after tool results that involve uteke
	pi.on("tool_result", async (event, ctx) => {
		if (!utekeAvailable) return;

		const content = event.content;
		const contentText = Array.isArray(content)
			? content.map((c: any) => c.type === "text" ? c.text : "").join("")
			: "";

		// Check if this was a uteke operation
		const isMemoryOp = /Memory stored|Memory forgotten|imported|cleanup|deleted/i.test(contentText);
		if (!isMemoryOp) return;

		// Refresh stats after a short delay to let the operation complete
		setTimeout(() => refreshStatus(ctx), 500);
	});

	// Register a command to manually refresh
	pi.registerCommand("uteke-stats", {
		description: "Refresh uteke memory stats in status bar",
		handler: async (_args, ctx) => {
			if (!utekeAvailable) {
				ctx.ui.notify("uteke is not installed or not in PATH", "error");
				return;
			}
			await refreshStatus(ctx);
			if (lastStats) {
				ctx.ui.notify(`🧠 ${lastStats.total_memories} memories (${lastStats.hot} hot, ${lastStats.warm} warm, ${lastStats.cold} cold)`, "info");
			}
		},
	});

	// Register a command to show detailed stats
	pi.registerCommand("uteke", {
		description: "Show detailed uteke memory statistics",
		handler: async (args, ctx) => {
			const cmd = args?.trim() || "stats";
			
			// Support subcommands: /uteke stats, /uteke aging, /uteke doctor
			if (["stats", "aging status", "doctor", "tags list"].includes(cmd)) {
				const { execFile } = await import("node:child_process");
				const fullCmd = cmd === "stats" ? "stats" : cmd;
				execFile("uteke", fullCmd.split(" "), { timeout: 15000 }, (error, stdout, stderr) => {
					if (error) {
						ctx.ui.notify(`uteke error: ${stderr || error.message}`, "error");
						return;
					}
					ctx.ui.notify(stdout.trim(), "info");
				});
			} else {
				ctx.ui.notify(`Usage: /uteke [stats|aging status|doctor|tags list]`, "info");
			}
		},
	});

	pi.on("session_shutdown", async (_event, _ctx) => {
		// Cleanup status
	});
}
