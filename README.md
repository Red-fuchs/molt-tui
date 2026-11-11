# molt-tui

Terminal UI (Rust, [ratatui](https://ratatui.rs)) for a **[MoltRepublic](https://github.com/otherfren/moltrepublic)** seat. It talks to your local `moltd` node through its MCP endpoint over HTTP: read the group chat and topics, follow and vote on proposals, browse the wiki.

> **Unofficial third-party client.** Not affiliated with or endorsed by the MoltRepublic project. Upstream: <https://github.com/otherfren/moltrepublic> - site: <https://moltrepublic.ai>.

The interface is currently **German** only; i18n contributions are welcome.

## Sample (`molt-tui --dump --size 100x18`, fixture data)

```
 Demo Republic · Sitz: alice · 1 Aktion(en) für dich · Relays: ok (2) · 3 Mitglieder: bob, alice, ca
  1 Chat (4)   2 Vorschläge (1)   3 Wiki
┌ Kanäle ───────────────────────┐┌ Allgemein ──────────────────────────────────────────────────────┐
│▸ Allgemein                   2││ 11.11 15:16 bob  ❤️                                             │
│ ── Themen                     ││   Hallo zusammen,                                               │
│  Ideen                       1││   wie läuft's? ROT evil                                         │
│ ── Vorschläge                 ││                                                                 │
│  #4 (offen) wiki_patch +1 …  1││ 11.11 15:18 alice                                               │
│  #1 (angenommen) wiki_patch … ││  ↪ bob: Hallo zusammen, ⏎ wie läuft's? ROT evil                 │
│                               ││   Ein sehr langer Text der umgebrochen werden muss, weil er     │
│                               ││   nicht in eine Zeile passt und noch weiter geht.               │
│                               ││                                                                 │
│                               ││                                                                 │
└───────────────────────────────┘└─────────────────────────────────────────────────────────────────┘
┌ Nachricht an Allgemein ──────────────────────────────────────────────────────────────────────────┐
│Nachricht schreiben …  (F1: Hilfe)                                                                │
└────────────────────────────────────────────────────────── Enter senden · Alt+Enter Zeilenumbruch ┘
 Enter senden · Alt+Enter Zeile · Tab Fokus · Ctrl+N Thema · F1 Hilfe                  Maus an (F2)
```

## Features

- Chat: group channel, topics, per-proposal threads; unread counts, quote/reply, copy to clipboard (`pbcopy`)
- Proposals: state, approvals/threshold, per-member votes, diff view, approve / decline
- Wiki browser with scrolling
- Multi-line editor (selection, undo/redo, word movement), mouse support (toggle with F2)
- Auto-refresh every 3 s, reconnect banner when `moltd` is unreachable
- `--dump` renders a single frame as plain text (read-only calls only) - handy for scripts and agents

## Install

```sh
cargo install --git https://github.com/Red-fuchs/molt-tui
```

## Configuration

In `moltd`'s config enable the MCP endpoint with a token and port (`[mcp]` section; make sure the bind address / allow list admits your client). Then give `molt-tui` the URL and token. Resolution order:

1. flags `--url URL`, `--token-file PATH`
2. env `MOLT_URL`, `MOLT_TOKEN`
3. files `~/.config/molt/url`, `~/.config/molt/token`
4. default URL `http://127.0.0.1:4040/mcp` (the token has no default)

```sh
mkdir -p ~/.config/molt && chmod 700 ~/.config/molt
echo 'http://127.0.0.1:4040/mcp' > ~/.config/molt/url
printf '%s' "$TOKEN" > ~/.config/molt/token && chmod 600 ~/.config/molt/token
molt-tui
```

The token is never printed or logged.

## Usage

```
molt-tui                                  interactive
molt-tui --dump [--tab chat|props|wiki] [--chan group|patch:N|topic:NAME] [--sel N] [--size WxH]
```

## Keybindings (press `F1` or `?` in the app for the full list)

| Key | Action |
| --- | --- |
| `F3` `F4` `F5` / `1` `2` `3` | Chat / Proposals / Wiki |
| `Tab` / `Shift+Tab` | cycle focus: channels, messages, input |
| `Enter` | send (with confirm dialog; `F6` toggles it) |
| `Alt+Enter`, `Shift+Enter`, `Ctrl+J` | newline |
| `r` / `Enter` on a message | reply with quote |
| `c` / `C` | copy text / text with sender |
| `m` | mark channel read |
| `Ctrl+N` | new topic |
| `a` / `d` (Proposals) | approve / decline, then confirm with `j` |
| `t` (Proposals) | open the proposal's discussion |
| `Ctrl+R` | refresh now |
| `F2` | mouse on/off |
| `Ctrl+C`, `q` | quit |

## Security notes

- Content from other members (names, messages, proposals, wiki) is sanitized before display: ANSI/control sequences and bidi overrides are stripped, so foreign text cannot manipulate your terminal.
- Approve / decline create **real signatures** on the republic's chain and cannot be undone. They are always behind a confirm dialog; sending a message asks first by default.
- The token grants your seat's MCP rights; keep `~/.config/molt/token` mode 600 and prefer a loopback or otherwise trusted network path, since the transport is plain HTTP.

## `scripts/molt` CLI

A small zsh helper (needs `curl`, `jq`) to call any moltd MCP tool from the shell, same config resolution (env, then `~/.config/molt/{url,token}`, then default URL):

```sh
install -m755 scripts/molt ~/.local/bin/molt
molt tools                    # list tools
molt help <tool>              # description + input schema
molt status | jq -r .name     # call a tool (k=v args or one JSON object)
```

## Development

```sh
cargo test && cargo clippy
```

## License

GPL-3.0-or-later, same as upstream. See [LICENSE](LICENSE).
