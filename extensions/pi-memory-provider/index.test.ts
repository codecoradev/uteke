// Run: bun test extensions/pi-memory-provider
// Checks the pi adapter against the real `uteke recall --json` output (#1593).
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { buildRecallArgs, parseRecallOutput } from "./index.ts";

const fixture = readFileSync(new URL("../fixtures/recall-cli.json", import.meta.url), "utf-8");

test("real CLI output yields a hit", () => {
	expect(parseRecallOutput(fixture)).toEqual(["caching keeps hot data close to the reader"]);
});

test("a nested memory object is still accepted", () => {
	expect(parseRecallOutput(JSON.stringify([{ score: 1, memory: { content: "nested" } }]))).toEqual(["nested"]);
});

test("garbage and empty output yield nothing", () => {
	for (const raw of ["", "  ", "not json", "{}", "null"]) {
		expect(parseRecallOutput(raw)).toEqual([]);
	}
});

test("the threshold flag is --min, never --min-score, and is omitted when off", () => {
	const off = buildRecallArgs("q", "ns", "6", "0", "proj");
	expect(off).not.toContain("--min");
	expect(off).not.toContain("--min-score");
	const on = buildRecallArgs("q", "ns", "6", "0.02", "");
	expect(on).toContain("--min");
	expect(on[on.indexOf("--min") + 1]).toBe("0.02");
	expect(on).not.toContain("--min-score");
	expect(buildRecallArgs("q", "ns", "6", "0", "proj")).toContain("project:proj");
});
