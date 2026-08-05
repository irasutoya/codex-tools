mod activation;
mod auth_center;
mod codex;
mod commands;
mod models;
mod network;
mod official_quota;
mod platform;
mod provider_http;
mod provider_sync;
mod proxy_import;
mod session_index;
mod state;
mod storage;

use activation::sync_active_codex_configuration;
use auth_center::AuthCenter;
use codex::ConfigManager;
use session_index::SessionIndex;
use state::{ActivationLock, ApiClient};
use storage::Store;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WindowEvent};

const TRAY_SHOW_ID: &str = "tray_show";
const TRAY_EXIT_ID: &str = "tray_exit";
const DEFAULT_WINDOW_WIDTH: f64 = 1180.0;
const DEFAULT_WINDOW_HEIGHT: f64 = 760.0;
const MIN_WINDOW_WIDTH: f64 = 360.0;
const MIN_WINDOW_HEIGHT: f64 = 520.0;

async fn sync_configured_provider(app: tauri::AppHandle) {
    let store = app.state::<Store>();
    let manager = app.state::<ConfigManager>();
    let activation = app.state::<ActivationLock>();
    let _guard = activation.0.lock().await;
    let _ = sync_active_codex_configuration(&store, &manager).await;
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        return;
    }
    let _ =
        tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("index.html".into()))
            .title("Codex Tools")
            .inner_size(DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT)
            .min_inner_size(MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT)
            .build();
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_main_window(app)
        }))
        .manage(Store::new().expect("无法初始化应用数据"))
        .manage(AuthCenter::default())
        .manage(ConfigManager::default())
        .manage(ActivationLock::default())
        .manage(ApiClient::default())
        .manage(SessionIndex::default())
        .setup(|app| {
            let show =
                MenuItem::with_id(app, TRAY_SHOW_ID, "显示 Codex Tools", true, None::<&str>)?;
            let exit = MenuItem::with_id(app, TRAY_EXIT_ID, "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &exit])?;
            let app_icon = app
                .default_window_icon()
                .cloned()
                .expect("application icon must be configured");
            TrayIconBuilder::new()
                .icon(app_icon)
                .menu(&menu)
                .tooltip("Codex Tools")
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    TRAY_SHOW_ID => show_main_window(app),
                    TRAY_EXIT_ID => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        show_main_window(tray.app_handle());
                    }
                })
                .build(app)?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(sync_configured_provider(handle));
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::dashboard::get_dashboard,
            commands::dashboard::get_settings_overview,
            commands::providers::get_provider_overview,
            commands::providers::save_provider,
            commands::providers::delete_provider,
            commands::providers::save_provider_account,
            commands::providers::delete_provider_account,
            commands::official_accounts::import_proxy_account,
            commands::official_accounts::start_openai_device_auth,
            commands::official_accounts::poll_openai_device_auth,
            commands::official_accounts::activate_openai_account,
            commands::official_accounts::delete_openai_account,
            commands::official_accounts::open_openai_device_page,
            commands::providers::test_provider,
            commands::official_accounts::refresh_official_account_quota,
            commands::official_accounts::refresh_all_official_quotas,
            commands::providers::preview_activation,
            commands::providers::apply_activation,
            commands::providers::activate_provider,
            commands::official_accounts::activate_official,
            commands::sessions::scan_codex_data,
            commands::sessions::repair_codex_data,
            commands::sessions::list_sessions,
            commands::dashboard::launch_codex,
        ])
        .build(tauri::generate_context!())
        .expect("Tauri 运行失败");
    app.run(|app, event| match event {
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } => show_main_window(app),
        _ => {
            let _ = app;
        }
    });
}
