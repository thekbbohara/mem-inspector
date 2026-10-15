//! Deterministic fixture generation and scan/hash measurements. Never cleans up.
use mem_core::{duplicates, scan, Cancellation, Limits};
use std::{env, fs, io::Write, path::Path, time::Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().collect();
    let command = args.get(1).map(String::as_str).unwrap_or("");
    let root = Path::new(
        args.get(2)
            .ok_or("usage: benchmark generate|scan|hash ABSOLUTE_ROOT [FILE_COUNT]")?,
    );
    if !root.is_absolute() {
        return Err("absolute fixture root required".into());
    }
    if command == "generate" {
        let count: usize = args
            .get(3)
            .map(|s| s.parse())
            .transpose()?
            .unwrap_or(20_000);
        if count == 0 || count > 80_000 {
            return Err("fixture count must be 1..80000".into());
        }
        fs::create_dir(root)?; // refuses to modify any existing fixture or owner directory
        for d in 0..100 {
            fs::create_dir(root.join(format!("d{d:03}")))?;
        }
        for i in 0..count {
            let mut bytes = [0_u8; 4096];
            let mut state = (i % 1000) as u64 + 1;
            for byte in &mut bytes {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                *byte = (state >> 32) as u8;
            }
            let mut file = fs::File::create_new(root.join(format!("d{:03}/f{i:06}.bin", i % 100)))?;
            file.write_all(&bytes)?;
        }
        for i in 0..8 {
            fs::File::create_new(root.join(format!("sparse{i}")))?.set_len(1 << 30)?;
        }
        #[cfg(target_os = "linux")]
        {
            std::os::unix::fs::symlink("d000", root.join("symlink"))?;
            fs::hard_link(root.join("d000/f000000.bin"), root.join("hardlink"))?;
        }
        println!(
            "generated regular={count} payload_bytes={} sparse=8x1GiB directories=100 seed=LCG-v1",
            count * 4096
        );
    } else {
        let start = Instant::now();
        let s = scan(root, &Limits::default(), &Cancellation::default())?;
        println!(
            "scan_ms={:.3} entries={} errors={} apparent={} allocated={} truncated={} cancelled={}",
            start.elapsed().as_secs_f64() * 1000.0,
            s.entries.len(),
            s.errors.len(),
            s.apparent,
            s.allocated,
            s.truncated,
            s.cancelled
        );
        if s.truncated || !s.errors.is_empty() {
            return Err("benchmark scan incomplete".into());
        }
        if command == "hash" {
            // Sparse files exceed the default per-file budget: benchmark intentionally
            // hashes only the deterministic 4 KiB payload files.
            let entries: Vec<_> = s
                .entries
                .into_iter()
                .filter(|e| e.kind == "file" && e.apparent == 4096)
                .collect();
            let start = Instant::now();
            let groups = duplicates(&entries, &Limits::default(), &Cancellation::default())?;
            println!(
                "hash_ms={:.3} groups={} paths={}",
                start.elapsed().as_secs_f64() * 1000.0,
                groups.len(),
                groups.iter().map(Vec::len).sum::<usize>()
            );
        } else if command != "scan" {
            return Err("unknown benchmark command".into());
        }
    }
    Ok(())
}
