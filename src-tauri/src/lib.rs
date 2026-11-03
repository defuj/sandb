pub mod commands;
pub mod core;

use std::path::PathBuf;

use tauri::menu::{Menu, MenuItem, MenuItemKind};
use tauri::{AppHandle, Emitter, Manager, Runtime, WindowEvent};

use crate::core::secrets::FileStore;
use crate::core::state::AppState;

const QUIT_ID: &str = "sandb-quit";

fn has_pending<R: Runtime, M: Manager<R>>(m: &M) -> bool {
    m.try_state::<AppState>()
        .is_some_and(|s| !s.sessions.pending_tab_ids().is_empty())
}

/// The default menu's Quit is the native `terminate:` action, which exits without a close request
/// (tao has no `applicationShouldTerminate:` hook), so it would skip the pending-transaction prompt.
/// Swap it for our own item with the same shortcut. Dock Quit still calls `terminate:` directly.
fn app_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let menu = Menu::default(app)?;
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    if let Some(MenuItemKind::Submenu(first)) = menu.items()?.into_iter().next() {
        // macOS: the app menu; Windows: File. Quit is the last item of both.
        let items = first.items()?;
        if let Some(MenuItemKind::Predefined(_)) = items.last() {
            first.remove_at(items.len() - 1)?;
            let quit = MenuItem::with_id(app, QUIT_ID, "Quit sandb", true, Some("CmdOrCtrl+Q"))?;
            first.append(&quit)?;
        }
    }
    Ok(menu)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let dir = match std::env::var_os("SANDB_DATA_DIR") {
                Some(d) => PathBuf::from(d),
                None => app.path().app_data_dir()?,
            };
            let secrets = FileStore::new(&dir);
            let state = AppState::new(dir, Box::new(secrets))?;
            app.manage(state);
            app.set_menu(app_menu(app.handle())?)?;
            Ok(())
        })
        // Quit asks the frontend first when a tab session could hold uncommitted work: the
        // window close (or menu Quit) is held back, the UI shows the same Commit / Rollback /
        // Cancel prompt as a tab close, and `quit_app` exits once the user decides.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && has_pending(window)
            {
                api.prevent_close();
                let _ = window.emit("sandb://quit-requested", ());
            }
        })
        .on_menu_event(|app, event| {
            if event.id() == QUIT_ID {
                if has_pending(app) {
                    let _ = app.emit("sandb://quit-requested", ());
                } else {
                    app.exit(0);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_connections,
            commands::create_connection,
            commands::update_connection,
            commands::duplicate_connection,
            commands::delete_connection,
            commands::test_connection,
            commands::list_databases,
            commands::list_schemas,
            commands::list_tables,
            commands::list_columns,
            commands::schema_snapshot,
            commands::table_ddl,
            commands::export_table_sql,
            commands::run_query,
            commands::cancel_query,
            commands::query_progress,
            commands::commit_transaction,
            commands::rollback_transaction,
            commands::tab_status,
            commands::close_tab_session,
            commands::quit_app,
            commands::export_csv,
            commands::guard_check,
            commands::classify_write,
            commands::run_ai_read,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
