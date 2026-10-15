fn main() {
    if std::env::var("PROFILE").as_deref() == Ok("release") && tauri_build::is_dev() {
        panic!(
            "Release builds must embed the frontend. Use npm run package or cargo build --release --features custom-protocol."
        );
    }
    tauri_build::build()
}
