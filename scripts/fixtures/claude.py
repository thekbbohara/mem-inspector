#!/usr/bin/python3
"""Disposable executable provider fixture. Never invokes Claude or the network.
The driver replaces FIXTURE_DIRECTORY with an absolute disposable directory.
"""

import json
import os
import sys
from pathlib import Path

fixture = Path("FIXTURE_DIRECTORY")
args = sys.argv[1:]
if args == ["--version"]:
    print("2.1.296 (Claude Code)")
elif args == ["--help"]:
    print(
        "--print --tools --safe-mode --restricted --strict-mcp-config --mcp-config "
        "--disable-slash-commands --no-chrome --no-session-persistence "
        "--setting-sources --settings --system-prompt --output-format --verbose "
        'Use "" to disable all tools; all customizations; Only use MCP servers from'
    )
elif args == ["auth", "status", "--json"]:
    print(
        json.dumps(
            {"loggedIn": True, "authMethod": "claude.ai", "apiProvider": "firstParty"}
        )
    )
else:
    data = json.load(sys.stdin)
    record = {
        "args": args,
        "stdin": data,
        "env_keys": sorted(os.environ),
        "cwd": os.getcwd(),
    }
    with (fixture / "provider-calls.jsonl").open("a") as log:
        log.write(json.dumps(record) + "\n")
    for flag in [
        "--print",
        "--safe-mode",
        "--restricted",
        "--strict-mcp-config",
        "--disable-slash-commands",
        "--no-chrome",
        "--no-session-persistence",
        "--verbose",
    ]:
        assert flag in args
    for flag, value in [
        ("--tools", ""),
        ("--setting-sources", ""),
        ("--mcp-config", '{"mcpServers":{}}'),
        ("--output-format", "stream-json"),
    ]:
        assert args[args.index(flag) + 1] == value
    settings = json.loads(args[args.index("--settings") + 1])
    assert settings["disableAllHooks"] is True
    assert settings["enabledPlugins"] == {
        "cc-plugin-agents-md@builtin": False,
        "cc-plugin-telemetry@builtin": False,
        "cc-plugin-plugin-authoring@builtin": False,
    }
    assert os.environ["CLAUDE_CODE_SAFE_MODE"] == "1"
    assert "ANTHROPIC_API_KEY" not in os.environ
    assert not list(Path.cwd().iterdir()), "Provider working directory must be empty"
    assert set(data) == {"total_bytes", "file_count", "categories"}
    assert type(data["total_bytes"]) is int and type(data["file_count"]) is int
    for category in data["categories"]:
        assert set(category) == {"kind", "bytes", "file_count"}
        assert category["kind"] in ["cache", "build", "downloads", "other"]
        assert type(category["bytes"]) is int and type(category["file_count"]) is int
    mode = (fixture / "provider-mode").read_text().strip()
    print(
        json.dumps(
            {
                "type": "system",
                "subtype": "init",
                "tools": [],
                "mcp_servers": [],
                "plugins": ["unsafe-fixture"] if mode == "unsafe" else [],
            }
        )
    )
    print(
        json.dumps(
            {
                "type": "result",
                "is_error": False,
                "result": "<b>FIXTURE ADVICE</b> & manual review only. Numeric aggregates cannot establish deletion safety.",
            }
        )
    )
