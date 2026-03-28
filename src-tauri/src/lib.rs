use tauri::{
    menu::{CheckMenuItem, MenuBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    ActivationPolicy, App, AppHandle, Manager, Runtime, Window, WindowEvent,
};

const TRAY_ID: &str = "trendwave-tray";
const MENU_OPEN_DASHBOARD: &str = "open-dashboard";
const MENU_TOGGLE_SCANNING: &str = "toggle-scanning";
const MENU_SETTINGS: &str = "open-settings";
const MENU_QUIT: &str = "quit";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayMenuAction {
    OpenDashboard,
    ToggleScanning,
    OpenSettings,
    Quit,
}

fn parse_tray_menu_action(id: &str) -> Option<TrayMenuAction> {
    match id {
        MENU_OPEN_DASHBOARD => Some(TrayMenuAction::OpenDashboard),
        MENU_TOGGLE_SCANNING => Some(TrayMenuAction::ToggleScanning),
        MENU_SETTINGS => Some(TrayMenuAction::OpenSettings),
        MENU_QUIT => Some(TrayMenuAction::Quit),
        _ => None,
    }
}

fn next_scanning_state(is_paused: bool) -> (bool, &'static str) {
    let next_state = !is_paused;
    let state_label = if next_state { "paused" } else { "resumed" };
    (next_state, state_label)
}

fn should_show_dashboard_from_tray_click(
    button: MouseButton,
    button_state: MouseButtonState,
) -> bool {
    button == MouseButton::Left && button_state == MouseButtonState::Up
}

fn show_dashboard<R: Runtime>(app: &AppHandle<R>) {
    match app.get_webview_window("main") {
        Some(window) => {
            if let Err(error) = window.show() {
                eprintln!("Failed to show the TrendWave dashboard: {error}");
            }

            if let Err(error) = window.unminimize() {
                eprintln!("Failed to restore the TrendWave dashboard: {error}");
            }

            if let Err(error) = window.set_focus() {
                eprintln!("Failed to focus the TrendWave dashboard: {error}");
            }
        }
        None => eprintln!("TrendWave could not find the main dashboard window."),
    }
}

fn hide_dashboard<R: Runtime>(window: &Window<R>) {
    if let Err(error) = window.hide() {
        eprintln!("Failed to hide the TrendWave dashboard: {error}");
    }
}

fn toggle_scanning_pause<R: Runtime>(pause_item: &CheckMenuItem<R>) {
    match pause_item.is_checked() {
        Ok(is_paused) => {
            let (next_state, state_label) = next_scanning_state(is_paused);

            if let Err(error) = pause_item.set_checked(next_state) {
                eprintln!("Failed to update the tray pause state: {error}");
                return;
            }

            println!("TrendWave scanning is now {state_label}.");
        }
        Err(error) => eprintln!("Failed to read the tray pause state: {error}"),
    }
}

fn build_tray<R: Runtime>(app: &mut App<R>) -> tauri::Result<()> {
    let pause_scanning_item = CheckMenuItem::with_id(
        app,
        MENU_TOGGLE_SCANNING,
        "Pause Scanning",
        true,
        false,
        None::<&str>,
    )?;

    let tray_menu = MenuBuilder::new(app)
        .text(MENU_OPEN_DASHBOARD, "Open Dashboard")
        .item(&pause_scanning_item)
        .text(MENU_SETTINGS, "Settings")
        .separator()
        .text(MENU_QUIT, "Quit")
        .build()?;

    // We clone the lightweight menu-item handle so the callback can own it
    // for the whole app lifetime without taking ownership of the native item.
    let pause_item_for_handler = pause_scanning_item.clone();

    let mut tray_builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&tray_menu)
        .tooltip("TrendWave")
        .show_menu_on_left_click(false)
        .on_menu_event(
            move |app, event| match parse_tray_menu_action(event.id().as_ref()) {
                Some(TrayMenuAction::OpenDashboard) => show_dashboard(app),
                Some(TrayMenuAction::ToggleScanning) => {
                    toggle_scanning_pause(&pause_item_for_handler)
                }
                Some(TrayMenuAction::OpenSettings) => {
                    println!("TrendWave settings will live in the dashboard in a later phase.");
                    show_dashboard(app);
                }
                Some(TrayMenuAction::Quit) => app.exit(0),
                None => {}
            },
        )
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button,
                button_state,
                ..
            } = event
            {
                if should_show_dashboard_from_tray_click(button, button_state) {
                    show_dashboard(tray.app_handle());
                }
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        tray_builder = tray_builder.icon(icon).icon_as_template(true);
    }

    let _ = tray_builder.build(app)?;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_result = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(ActivationPolicy::Accessory);

            build_tray(app).map_err(|error| -> Box<dyn std::error::Error> { Box::new(error) })?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();

                // Tauri gives us a shared reference to the existing window here,
                // which is enough because hiding the window does not require ownership.
                hide_dashboard(window);
            }
        })
        .run(tauri::generate_context!());

    if let Err(error) = app_result {
        eprintln!("TrendWave failed to start: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_tray_menu_actions() {
        assert_eq!(
            parse_tray_menu_action(MENU_OPEN_DASHBOARD),
            Some(TrayMenuAction::OpenDashboard)
        );
        assert_eq!(
            parse_tray_menu_action(MENU_TOGGLE_SCANNING),
            Some(TrayMenuAction::ToggleScanning)
        );
        assert_eq!(
            parse_tray_menu_action(MENU_SETTINGS),
            Some(TrayMenuAction::OpenSettings)
        );
        assert_eq!(
            parse_tray_menu_action(MENU_QUIT),
            Some(TrayMenuAction::Quit)
        );
    }

    #[test]
    fn ignores_unknown_tray_menu_actions() {
        assert_eq!(parse_tray_menu_action("not-a-real-menu-item"), None);
    }

    #[test]
    fn toggling_scanning_from_running_pauses_it() {
        assert_eq!(next_scanning_state(false), (true, "paused"));
    }

    #[test]
    fn toggling_scanning_from_paused_resumes_it() {
        assert_eq!(next_scanning_state(true), (false, "resumed"));
    }

    #[test]
    fn only_left_button_release_opens_the_dashboard() {
        assert!(should_show_dashboard_from_tray_click(
            MouseButton::Left,
            MouseButtonState::Up
        ));

        assert!(!should_show_dashboard_from_tray_click(
            MouseButton::Left,
            MouseButtonState::Down
        ));
        assert!(!should_show_dashboard_from_tray_click(
            MouseButton::Right,
            MouseButtonState::Up
        ));
        assert!(!should_show_dashboard_from_tray_click(
            MouseButton::Middle,
            MouseButtonState::Up
        ));
    }
}
