# Installing Uteke as an MCP server (for AI agents)

Uteke is long-term memory for AI agents. The MCP server is the `uteke-mcp`
binary: it speaks stdio, runs fully offline and needs no API key.

## 1. Install the binary

Pick one; all of them put `uteke` and `uteke-mcp` on `PATH`.

```bash
# macOS (Apple Silicon) and Linux
brew install codecoradev/tap/uteke

# any platform with a Rust toolchain
cargo install uteke-cli uteke-mcp
```

Windows or no toolchain: download the archive for your platform from
https://github.com/codecoradev/uteke/releases and put `uteke.exe` and
`uteke-mcp.exe` on `PATH`.

Check it: `uteke --version` prints a version. Do not run `uteke-mcp --version`;
it starts the server and waits on stdin.

## 2. Register the MCP server

Add this to the client's MCP configuration (for Claude Code and Cursor that is
`.mcp.json`; for Cline it is the MCP settings file):

```json
{
  "mcpServers": {
    "uteke": {
      "command": "uteke-mcp"
    }
  }
}
```

No environment variables are required. If the client cannot find `uteke-mcp`
on its `PATH`, give the absolute path to the binary as `command` (`which uteke-mcp`).

## 3. First run

The first call downloads the embedding model (about 200 MB, one time) into
`~/.codecora/uteke/`. After that, memory operations make no network calls.

Verify with the `uteke_remember` tool, then `uteke_recall`.

## Hosted alternative (no install)

Uteke Cloud runs the same engine: add `https://uteke.app/mcp` as a remote
Streamable HTTP server and sign in with OAuth when the client asks.

More: https://github.com/codecoradev/uteke/blob/develop/docs/mcp.md
