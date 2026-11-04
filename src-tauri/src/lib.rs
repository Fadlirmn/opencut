pub mod audio;
pub mod binaries;
pub mod commands;
pub mod cutlist;
pub mod export_fcpxml;
pub mod export_mp4;
pub mod llm;
pub mod logstore;
pub mod oauth;
pub mod probe;
pub mod queue;
pub mod settings;
pub mod subtitle;
pub mod sync;
pub mod timecode;
pub mod timeline;
pub mod transcribe;
pub mod vad;
pub mod waveform;

/// Excluded from the lib's own test build on purpose. `generate_context!` is a
/// proc macro that validates tauri.conf.json at compile time, including that
/// `frontendDist` exists — so leaving it in would make `cargo test` refuse to
/// compile until someone had run a frontend build. None of the unit tests
/// touch the Tauri runtime. The binary target still compiles the lib without
/// cfg(test), so the real app is unaffected.
#[cfg(not(test))]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use commands::AppState;
    use tauri::Manager;

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            // AppState needs the app-data dir (for settings.json/logs.json),
            // which only exists once the app handle does — hence building it
            // here instead of via `.manage(AppState::default())`.
            let app_data_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("resolving app data dir: {e}"))?;
            app.manage(AppState::new(app_data_dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::open_video,
            commands::open_track,
            commands::compute_waveform,
            commands::detect_silence,
            commands::export_mp4,
            commands::cancel_export,
            commands::cancel_detect,
            commands::cancel_waveform,
            commands::export_fcpxml,
            commands::diagnostic_info,
            commands::reveal_in_finder,
            commands::generate_subtitles,
            commands::cancel_subtitles,
            commands::save_subtitles,
            commands::queue_add,
            commands::queue_snapshot,
            commands::queue_remove,
            commands::queue_retry,
            commands::queue_clear_done,
            commands::queue_start,
            commands::get_settings,
            commands::save_settings,
            commands::log_list,
            commands::llm_status,
            commands::llm_connect,
            commands::llm_cancel_connect,
            commands::llm_disconnect,
            commands::llm_test,
        ])
        .run(tauri::generate_context!())
        .expect("error while running autocut");
}
