pub mod commands;
pub mod store;
pub mod transcribe;

use tauri::Manager;

use store::Store;

const DATABASE_FILE: &str = "earshot.db";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            app.manage(Store::open(&data_dir.join(DATABASE_FILE))?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::meetings::create_meeting,
            commands::meetings::list_meetings,
            commands::meetings::get_meeting,
            commands::meetings::rename_meeting,
            commands::meetings::update_notes,
            commands::meetings::delete_meeting,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
