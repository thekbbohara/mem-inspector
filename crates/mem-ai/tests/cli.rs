#![cfg(unix)]
use mem_ai::{
    advice, discover, propose, Category, CategoryKind, Error, Evidence, ProposalItem, ScanSummary,
    UsageWindow,
};
use std::{fs, os::unix::fs::PermissionsExt};
#[test]
fn fixture_discovery_invocation_and_failure_modes() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("calls.jsonl");
    let script = format!(
        r#"#!/usr/bin/python3
import sys,json,os
with open({log:?},'a') as f: f.write(json.dumps({{'args':sys.argv[1:],'env':list(os.environ),'cwd':os.getcwd()}})+'\n')
a=sys.argv[1:]
if a==['--version']: print('2.1.299 (Claude Code)')
elif a==['--help']: print('--print --tools --safe-mode --restricted --strict-mcp-config --mcp-config --disable-slash-commands --no-chrome --no-session-persistence --setting-sources --settings --system-prompt --output-format --verbose Use "" to disable all tools; all customizations; Only use MCP servers from')
elif a==['auth','status','--json']: print(json.dumps({{'loggedIn':True,'authMethod':'claude.ai','apiProvider':'firstParty'}}))
else:
 settings=json.loads(a[a.index('--settings')+1])
 assert settings['disableAllHooks'] and len(settings['enabledPlugins'])==3 and all(v is False for v in settings['enabledPlugins'].values())
 data=json.load(sys.stdin)
 if isinstance(data,list):
  for item in data:
   assert set(item)=={{'id','bytes','evidence'}} and type(item['id']) is int and type(item['bytes']) is int
   assert item['evidence'] in ['cache_intent','user_confirmed','unknown']
  result=json.dumps({{'selected':[i['id'] for i in data if i['evidence']!='unknown']}})
 else:
  assert set(data)=={{'total_bytes','file_count','categories'}}
  result='Review cache and build artifacts manually; aggregates do not establish deletion safety.'
 with open({log:?},'a') as f: f.write(json.dumps({{'args':a,'env':list(os.environ),'cwd':os.getcwd(),'stdin':data}})+'\n')
 print(json.dumps({{'type':'system','subtype':'init','tools':[],'mcp_servers':[],'plugins':[]}}))
 print(json.dumps({{'type':'result','is_error':False,'result':result}}))
"#,
        log = log.to_str().unwrap()
    );
    let cli = dir.path().join("claude");
    fs::write(&cli, &script).unwrap();
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
    let old = std::env::var_os("PATH");
    std::env::set_var("PATH", dir.path());
    std::env::set_var("ANTHROPIC_API_KEY", "fixture-never-forward");
    let summary = ScanSummary {
        total_bytes: 100,
        file_count: 2,
        categories: vec![Category {
            kind: CategoryKind::Cache,
            bytes: 50,
            file_count: 1,
        }],
    };
    assert_eq!(
        advice("claude", &summary, false).unwrap_err(),
        Error::ConsentRequired
    );
    assert!(!log.exists());
    let providers = discover();
    assert!(providers[0].installed && providers[0].usable);
    assert!(providers[0].reason.contains("Claude 2.1"));
    assert!(providers[0]
        .reason
        .contains("runtime checked on each request"));
    assert!(!providers[1].usable);
    assert_eq!(providers[1].reason, "CLI not found on application PATH");
    // An installed CLI without an adapter must not be presented as unauthenticated.
    fs::write(
        dir.path().join("codex"),
        "#!/usr/bin/python3\nprint('codex-cli 0.162.1')\n",
    )
    .unwrap();
    fs::set_permissions(dir.path().join("codex"), fs::Permissions::from_mode(0o700)).unwrap();
    let provider = &discover()[1];
    assert!(provider.installed && !provider.usable);
    assert!(provider.reason.contains("required isolation flags"));
    for (changed, reason) in [
        (
            script.replace("2.1.299", "2.2.0"),
            "Unsupported Claude CLI version",
        ),
        (
            script.replace("2.1.299", "2.1.295"),
            "Unsupported Claude CLI version",
        ),
        (
            script.replace("--tools ", "--tools-extra "),
            "required isolation flags",
        ),
        (
            script.replace("all customizations", "some customizations"),
            "isolation semantics",
        ),
        (
            script.replace("'loggedIn':True", "'loggedIn':False"),
            "login is not verified",
        ),
        (
            script.replace("'authMethod':'claude.ai'", "'authMethod':'apiKey'"),
            "API billing is unsupported",
        ),
        (
            script.replace("'apiProvider':'firstParty'", "'apiProvider':'bedrock'"),
            "API billing is unsupported",
        ),
    ] {
        fs::write(&cli, changed).unwrap();
        let provider = &discover()[0];
        assert!(provider.installed && !provider.usable);
        assert!(provider.reason.contains(reason), "{}", provider.reason);
        assert!(advice("claude", &summary, true)
            .unwrap_err()
            .to_string()
            .contains(reason));
    }
    // Discovery and rejected requests never trigger inference.
    let calls = fs::read_to_string(&log).unwrap();
    assert!(calls.lines().all(|line| !line.contains("--print")));
    fs::write(&cli, script.replace("2.1.299", "2.1.296")).unwrap();
    assert!(discover()[0].usable);
    fs::write(&cli, &script).unwrap();
    let result = advice("claude", &summary, true).unwrap();
    assert!(result.text.contains("manually"));
    assert!(advice("codex", &summary, true)
        .unwrap_err()
        .to_string()
        .contains("required isolation flags"));
    let items = vec![
        ProposalItem {
            id: 0,
            bytes: 100,
            evidence: Evidence::CacheIntent,
        },
        ProposalItem {
            id: 1,
            bytes: 999,
            evidence: Evidence::Unknown,
        },
        ProposalItem {
            id: 2,
            bytes: 50,
            evidence: Evidence::UserConfirmed,
        },
    ];
    assert_eq!(
        propose("claude", &items, false).unwrap_err(),
        Error::ConsentRequired
    );
    assert_eq!(
        propose("claude", &items, true).unwrap().selected,
        vec![0, 2]
    );
    fs::write(&cli, script.replace("'is_error':False", "'is_error':True")).unwrap();
    assert_eq!(
        propose("claude", &items, true).unwrap_err(),
        Error::ProviderError
    );
    let quota_script = script.replace("'is_error':False", "'is_error':True").replace(
        " print(json.dumps({'type':'result'",
        " print(json.dumps({'type':'rate_limit_event','rate_limit_info':{'status':'rejected','rateLimitType':'seven_day','isUsingOverage':False,'overageStatus':'rejected'}}))\n print(json.dumps({'type':'result'",
    );
    fs::write(&cli, quota_script).unwrap();
    assert_eq!(
        propose("claude", &items, true).unwrap_err(),
        Error::UsageLimit(UsageWindow::Weekly)
    );
    fs::write(&cli, script.replace("'tools':[]", "'tools':['Read']")).unwrap();
    assert_eq!(
        propose("claude", &items, true).unwrap_err(),
        Error::InvalidResponse
    );
    // Reject an unsafe startup while the provider is still running, rather than
    // waiting for its model result or timeout. Test tools, MCP and plugins.
    for field in ["tools", "mcp_servers", "plugins"] {
        let changed = script.replace(
            &format!("'{field}':[]"),
            &format!("'{field}':['unsafe']"),
        ).replace(
            " print(json.dumps({'type':'result'",
            " sys.stdout.flush()\n import time\n time.sleep(3)\n print(json.dumps({'type':'result'",
        );
        fs::write(&cli, changed).unwrap();
        let start = std::time::Instant::now();
        assert_eq!(
            propose("claude", &items, true).unwrap_err(),
            Error::InvalidResponse
        );
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "unsafe startup was not terminated promptly"
        );
    }
    fs::write(&cli, &script).unwrap();
    let calls = fs::read_to_string(&log).unwrap();
    let records: Vec<serde_json::Value> = calls
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    for r in &records {
        assert!(!r["env"]
            .as_array()
            .unwrap()
            .iter()
            .any(|k| k == "ANTHROPIC_API_KEY"));
        assert_ne!(
            r["cwd"].as_str().unwrap(),
            std::env::current_dir().unwrap().to_str().unwrap()
        );
    }
    let args = records.last().unwrap()["args"].as_array().unwrap();
    for pair in [
        ["--tools", ""],
        ["--mcp-config", "{\"mcpServers\":{}}"],
        ["--setting-sources", ""],
    ] {
        assert!(args.windows(2).any(|w| w[0] == pair[0] && w[1] == pair[1]));
    }
    fs::write(
        &cli,
        "#!/usr/bin/python3\nimport sys\nprint('unsupported')\n",
    )
    .unwrap();
    assert!(discover()[0].installed);
    assert!(!discover()[0].usable);
    assert!(advice("claude", &summary, true)
        .unwrap_err()
        .to_string()
        .contains("required isolation flags"));
    if let Some(v) = old {
        std::env::set_var("PATH", v);
    } else {
        std::env::remove_var("PATH");
    }
    std::env::remove_var("ANTHROPIC_API_KEY");
}
