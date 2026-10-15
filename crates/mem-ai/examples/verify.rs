use mem_ai::{advice, discover, Category, CategoryKind, ScanSummary};
fn main() {
    println!("{}", serde_json::to_string(&discover()).unwrap());
    if std::env::args().any(|a| a == "--infer-with-consent") {
        let s = ScanSummary {
            total_bytes: 1_000_000_000,
            file_count: 1000,
            categories: vec![
                Category {
                    kind: CategoryKind::Cache,
                    bytes: 400_000_000,
                    file_count: 300,
                },
                Category {
                    kind: CategoryKind::Build,
                    bytes: 300_000_000,
                    file_count: 400,
                },
            ],
        };
        match advice("claude", &s, true) {
            Ok(a) => println!("{}", serde_json::to_string(&a).unwrap()),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    }
}
