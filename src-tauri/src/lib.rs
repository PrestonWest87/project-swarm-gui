mod store;
mod sync;
mod crypto;
mod kex;
mod network;

use std::sync::Mutex;
use tauri::Manager;
use tokio::sync::mpsc;

pub enum NetworkCommand {
    SendMessage(String),
    GenerateInvite,
    JoinRoom(String),
    DiscoverPeers, // [NEW]
    Whisper { peer_id: String, message: String }, // [NEW]
}

struct AppState {
    network_tx: Mutex<Option<mpsc::Sender<NetworkCommand>>>,
}

#[tauri::command]
async fn send_message(message: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
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

// [NEW] Trigger a global DHT search
#[tauri::command]
async fn discover_peers(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let tx = { state.network_tx.lock().unwrap().clone() };
    if let Some(tx) = tx {
        tx.send(NetworkCommand::DiscoverPeers).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

// [NEW] Trigger an E2E Encrypted Whisper
#[tauri::command]
async fn whisper_peer(peer_id: String, message: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let tx = { state.network_tx.lock().unwrap().clone() };
    if let Some(tx) = tx {
        tx.send(NetworkCommand::Whisper { peer_id, message }).await.map_err(|e| e.to_string())?;
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
            
            tauri::async_runtime::spawn(async move {
                if let Err(e) = network::start_swarm(app_handle, rx).await {
                    eprintln!("[ERROR] Network thread crashed: {}", e);
                }
            });

            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        // [UPDATED] Added the two new commands to the invoke handler
        .invoke_handler(tauri::generate_handler![
            send_message, 
            generate_invite, 
            join_room, 
            discover_peers, 
            whisper_peer
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}