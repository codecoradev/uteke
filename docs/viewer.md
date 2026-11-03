# Viewer

`uteke view` opens a read-only page in your browser: memories (with a namespace
filter), namespaces with counts, rooms, and store health. It is for looking at
what is stored; nothing in it can change the store.

```bash
uteke view                 # http://127.0.0.1:8768/?token=...
uteke view --port 9000     # another port (0 picks a free one)
```

The command prints one URL. Open it; the token in it is the key, so treat the URL
like a password for as long as the viewer runs. Press Ctrl+C to stop.

It runs `uteke-serve --viewer`, so the full install bundle (or `uteke-serve` on
`PATH`) is needed. It uses the same store as every other command
(`UTEKE_HOME`, `uteke.toml`) and does not load the embedding model.

## What protects it

A loopback port is reachable by any local process, and by a web page through DNS
rebinding, so loopback alone is not the control. The viewer applies these, in
order, to every request:

| Check | Refused with |
|---|---|
| `Host` is not `127.0.0.1:<port>` or `localhost:<port>` (the same host guard the API server uses, plus a port check) | 403 |
| No per-launch token, or the wrong one (cookie, `X-Uteke-Viewer-Token`, or bearer; the launch URL carries it once, on `/` only) | 401 |
| Method other than `GET` | 405 |
| Path other than the page and the read routes below | 404 |

- **Read-only by construction.** The viewer hands a request to the API router only
  for `/health`, `/namespaces`, `/stats`, `/room/list`, `/recent` and `/memory`,
  and only while the route registry marks that row `writes = false`
  (`api_registry.rs`). A write route cannot be reached, and a `POST` to any path is
  refused before any routing.
- **No CORS.** No `Access-Control-*` header is ever sent, whatever `uteke.toml`
  configures for the API server. The page is same-origin and sets
  `Content-Security-Policy` (`default-src 'none'`, nonce-locked script and style),
  `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer` and `Cache-Control:
  no-store`.
- **Stored text is data.** The page puts every stored string into the DOM with
  `textContent`; it never parses memory content as HTML. A memory that contains
  `<script>` shows those characters.
- **Binding.** The default is `127.0.0.1`. `--host` with a non-loopback address is
  refused unless the server auth token (`UTEKE_AUTH_TOKEN` or `--auth-token`) is
  set; the viewer token is still required on top.
- The token is random per launch (two v4 UUIDs, 64 hex digits), compared in constant
  time, and printed only on stdout, never logged. After the first load the page
  moves it to an `HttpOnly; SameSite=Strict` cookie and removes it from the address
  bar.

## Not part of the API

The viewer is a separate listener, not a route of `uteke-serve`. Its paths are not in
the API registry, so CORE and LAB are unchanged, and a running `uteke-serve` is not
affected. The page's design tokens are the plain-CSS `@codecora/tokens` (`uteke` and
`uteke-dark` themes), vendored as one file: there is no build step and no framework.
