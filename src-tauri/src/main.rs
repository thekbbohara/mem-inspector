mod advisor;
mod bridge;
mod evidence;
mod hostenv;
mod placement;
mod settings;
mod space;
mod tools;
use tauri_plugin_dialog::DialogExt;
#[tauri::command]
async fn request(
    app: tauri::AppHandle,
    state: tauri::State<'_, bridge::Bridge>,
    request: bridge::Request,
) -> Result<serde_json::Value, String> {
    if matches!(request.action, bridge::Action::Choose) {
        if let Some(root) = state.fixture_root() {
            return Ok(serde_json::json!({"root": root, "fixture": true}));
        }
        let path = tauri::async_runtime::spawn_blocking(move || {
            app.dialog().file().blocking_pick_folder()
        })
        .await
        .map_err(|e| e.to_string())?;
        return Ok(serde_json::json!({"root": path.map(|p| p.to_string())}));
    }
    if matches!(request.action, bridge::Action::Pick) {
        // Folder for personal files; the backend checks it on SetArchive.
        let path = tauri::async_runtime::spawn_blocking(move || {
            app.dialog()
                .file()
                .set_title("Where should personal files go?")
                .blocking_pick_folder()
        })
        .await
        .map_err(|e| e.to_string())?;
        return Ok(serde_json::json!({"path": path.map(|p| p.to_string())}));
    }
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || shared.request(request))
        .await
        .map_err(|e| e.to_string())?
}
fn main() {
    let bridge = bridge::Bridge::from_env().expect("invalid fixture configuration");
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(bridge)
        .invoke_handler(tauri::generate_handler![request])
        .run(tauri::generate_context!())
        .expect("Tauri launch failed");
}

#[cfg(all(test, feature = "custom-protocol"))]
mod production_tests {
    #[test]
    fn production_context_embeds_frontend_without_a_dev_server() {
        assert!(!tauri::is_dev());
        let context: tauri::Context<tauri::Wry> = tauri::generate_context!();
        let index = context
            .assets()
            .get(&"index.html".into())
            .expect("embedded frontend");
        let html = std::str::from_utf8(&index).expect("UTF-8 frontend");
        assert!(html.contains("id=\"root\""));
        assert!(html.contains("assets/"));
        for asset in html.split('"').filter(|part| part.starts_with("./assets/")) {
            assert!(
                context.assets().get(&asset[2..].into()).is_some(),
                "missing embedded asset: {asset}"
            );
        }
    }
}
