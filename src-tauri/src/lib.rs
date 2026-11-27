mod commands;
mod net;
mod protocol;
mod state;

use std::sync::{atomic::Ordering, Arc};
use tauri::Manager;
use tokio::net::TcpListener;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_local_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let profile = state::load_profile(&data_dir);
            let shared = Arc::new(state::Shared::new(app.handle().clone(), data_dir, profile));

            // Prefer the well-known game port (needed for "connect by IP"); fall back to any free port.
            let listener = tauri::async_runtime::block_on(async {
                match TcpListener::bind(("0.0.0.0", protocol::GAME_PORT)).await {
                    Ok(l) => Ok(l),
                    Err(_) => TcpListener::bind(("0.0.0.0", 0)).await,
                }
            })?;
            shared.game_port.store(listener.local_addr()?.port(), Ordering::SeqCst);

            app.manage(shared.clone());
            tauri::async_runtime::spawn(net::run_server(shared.clone(), listener));
            tauri::async_runtime::spawn(net::run_discovery(shared.clone()));
            tauri::async_runtime::spawn(net::run_clock_watch(shared.clone()));
            // Warm the ECO index off the UI thread.
            std::thread::spawn(|| {
                let _ = cheslan_core::eco::entries();
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_profile,
            commands::save_profile,
            commands::list_peers,
            commands::challenge,
            commands::challenge_ip,
            commands::respond_challenge,
            commands::cancel_challenge,
            commands::get_phase,
            commands::get_session,
            commands::make_move,
            commands::resign,
            commands::offer_draw,
            commands::respond_draw,
            commands::send_chat,
            commands::request_rematch,
            commands::respond_rematch,
            commands::leave_game,
            commands::get_pgn,
            commands::get_review_input,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ChesLan");
}
