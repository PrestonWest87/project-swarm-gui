mod store;
mod sync;
mod crypto;
mod kex;
mod network;

use std::sync::Mutex;
use tauri::Manager;
use tokio::sync::mpsc;

// Commands sent from the Svelte GUI to the Network Thread
pub enum NetworkCommand {
    SendMessage(String),
    GenerateInvite,
    JoinRoom(String),
}

// Holds the transmitter so Tauri commands can access it
struct AppState {
    network_tx: Mutex<Option<mpsc::Sender<NetworkCommand>>>,
}

#[tauri::command]
async fn send_message(message: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    // Isolate the lock in a block so it drops BEFORE the await
    let tx = { state.network_tx.lock().unwrap().clone() };
    if let Some(tx) = tx {
        tx.send(NetworkCommand::SendMessage(message)).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn generate_invite(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let tx = { state.network_tx.lock().unwrap().clone() };
    if let Some(tx) = tx {
        tx.send(NetworkCommand::GenerateInvite).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn join_room(hash: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let tx = { state.network_tx.lock().unwrap().clone() };
    if let Some(tx) = tx {
        tx.send(NetworkCommand::JoinRoom(hash)).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            network_tx: Mutex::new(None),
        })
        .setup(|app| {
            let (tx, rx) = mpsc::channel::<NetworkCommand>(100);
            
            let state = app.state::<AppState>();
            *state.network_tx.lock().unwrap() = Some(tx);

            let app_handle = app.handle().clone();
            
            // Spawn the massive Post-Quantum Swarm engine in the background
            tauri::async_runtime::spawn(async move {
                if let Err(e) = network::start_swarm(app_handle, rx).await {
                    eprintln!("[ERROR] Network thread crashed: {}", e);
                }
            });

            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![send_message, generate_invite, join_room])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}