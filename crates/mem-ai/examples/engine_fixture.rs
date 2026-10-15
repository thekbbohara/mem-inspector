//! Minimal live verification. Numeric fixture only; never scans or cleans files.
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("discover") {
        println!(
            "{}",
            serde_json::to_string_pretty(&mem_ai::discover()).unwrap()
        );
        return;
    }
    let name = args.get(1).expect("engine name or discover");
    let settings = mem_ai::EngineSettings {
        model: args.get(2).cloned().unwrap_or_default(),
        account: args.get(3).cloned().unwrap_or_default(),
    };
    let result = mem_ai::propose_with(
        name,
        &[
            mem_ai::ProposalItem {
                id: 0,
                bytes: 4096,
                evidence: mem_ai::Evidence::CacheIntent,
            },
            mem_ai::ProposalItem {
                id: 1,
                bytes: 8192,
                evidence: mem_ai::Evidence::Unknown,
            },
        ],
        true,
        Some(&settings),
    );
    match result {
        Ok(proposal) => println!(
            "{}",
            serde_json::json!({"engine":name,"live":true,"proposal":proposal})
        ),
        Err(error) => {
            println!(
                "{}",
                serde_json::json!({"engine":name,"live":true,"blocker":error.to_string()})
            );
            std::process::exit(1);
        }
    }
}
