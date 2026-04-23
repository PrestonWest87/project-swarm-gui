use crate::{crypto, kex, store, sync};
use base64::{Engine as _, engine::general_purpose::URL_SAFE};
use futures::StreamExt;
use libp2p::{
    autonat, dcutr, gossipsub, identify, identity, kad, mdns, noise, relay, request_response,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, upnp, yamux, Multiaddr, PeerId,
};
use pqcrypto_traits::kem::PublicKey;
use rand_core::RngCore;
use std::collections::{hash_map::DefaultHasher, HashSet};
use std::error::Error;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::Emitter;

const BOOTSTRAP_NODES: &[&str] = &[
    "/dnsaddr/bootstrap.libp2p.io/p2p/QmNnooDu7bfjPFoTZYxMNLWUQJyrVwtbZg5gBMjTezGAJN",
    "/dnsaddr/bootstrap.libp2p.io/p2p/QmQCU2EcMqAqQPR2i9bChDtGNJchTbq5TbXBPxW8V92uMb",
    "/ip4/104.131.131.82/tcp/4001/p2p/QmaCpDMGvV2BGHeYERUEnRQAwe3N8SzbUtfsmvsqQLuvuJ",
];

// Struct for Base64 encoded invites (now signed and verified)
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct FatInvite {
    sender_x25519_pub: Vec<u8>,
    sender_mlkem_pub: Vec<u8>,
    signature: Vec<u8>,
    addrs: Vec<String>,
    topic: String,
}

#[derive(NetworkBehaviour)]
struct SwarmProtocol {
    gossipsub: gossipsub::Behaviour,
    mdns: mdns::tokio::Behaviour,
    kademlia: kad::Behaviour<kad::store::MemoryStore>,
    req_res: request_response::cbor::Behaviour<sync::SyncRequest, sync::SyncResponse>,
    kex: request_response::cbor::Behaviour<kex::KexRequest, kex::KexResponse>,
    identify: identify::Behaviour,
    autonat: autonat::Behaviour,
    dcutr: dcutr::Behaviour,
    relay_client: relay::client::Behaviour,
    relay_server: relay::Behaviour, // ADDED: Emergent server routing capability
    upnp: upnp::tokio::Behaviour,
}

pub async fn start_swarm(
    app: tauri::AppHandle,
    mut rx: tokio::sync::mpsc::Receiver<crate::NetworkCommand>,
) -> Result<(), Box<dyn Error>> {
    let my_crypto_id = crypto::HybridIdentity::generate();
    let db = Arc::new(Mutex::new(store::Store::new(my_crypto_id.derive_storage_key()).expect("Failed to init SQLite")));

    let key_path = "swarm_network_key.bin";
    let local_key = match std::fs::read(key_path) {
        Ok(bytes) => identity::Keypair::from_protobuf_encoding(&bytes).unwrap(),
        Err(_) => {
            let new_key = identity::Keypair::generate_ed25519();
            std::fs::write(key_path, new_key.to_protobuf_encoding().unwrap()).unwrap();
            new_key
        }
    };

    let local_peer_id = PeerId::from(local_key.public());
    let local_author_id = local_peer_id.to_string();

    let message_id_fn = |message: &gossipsub::Message| {
        let mut s = DefaultHasher::new();
        message.data.hash(&mut s);
        gossipsub::MessageId::from(s.finish().to_string())
    };

    let gossipsub_config = gossipsub::ConfigBuilder::default()
        .heartbeat_interval(Duration::from_secs(5)) // Sped up heartbeat
        .validation_mode(gossipsub::ValidationMode::Strict)
        .message_id_fn(message_id_fn)
        .build()
        .unwrap();

    let mut gossipsub_behaviour = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(local_key.clone()),
        gossipsub_config,
    ).unwrap();

    let mut current_topic = "swarm-alpha".to_string();
    let initial_topic = gossipsub::IdentTopic::new(current_topic.clone());
    gossipsub_behaviour.subscribe(&initial_topic).unwrap();

    let mdns_behaviour = mdns::tokio::Behaviour::new(mdns::Config::default(), local_key.public().to_peer_id())?;
    
    let mut kad_config = kad::Config::default();
    kad_config.set_query_timeout(Duration::from_secs(15));
    kad_config.set_replication_factor(std::num::NonZeroUsize::new(2).unwrap());
    
    let kad_store = kad::store::MemoryStore::new(local_peer_id);
    let mut kad_behaviour = kad::Behaviour::with_config(local_peer_id, kad_store, kad_config);
    kad_behaviour.set_mode(Some(kad::Mode::Server));

    let req_res_behaviour = request_response::cbor::Behaviour::new(
        [(sync::SYNC_PROTOCOL_NAME, request_response::ProtocolSupport::Full)],
        request_response::Config::default(),
    );

    let kex_behaviour = request_response::cbor::Behaviour::new(
        [(kex::KEX_PROTOCOL_NAME, request_response::ProtocolSupport::Full)],
        request_response::Config::default(),
    );

    let identify_behaviour = identify::Behaviour::new(identify::Config::new(
        "/project-swarm/1.0.0".into(),
        local_key.public(),
    ));

    let autonat_behaviour = autonat::Behaviour::new(local_peer_id, autonat::Config::default());
    let (_relay_transport, relay_client_behaviour) = relay::client::new(local_peer_id);
    let relay_server_behaviour = relay::Behaviour::new(local_peer_id, relay::Config::default()); // Added
    let dcutr_behaviour = dcutr::Behaviour::new(local_peer_id);
    let upnp_behaviour = upnp::tokio::Behaviour::default();

    let behaviour = SwarmProtocol {
        gossipsub: gossipsub_behaviour,
        mdns: mdns_behaviour,
        kademlia: kad_behaviour,
        req_res: req_res_behaviour,
        kex: kex_behaviour,
        identify: identify_behaviour,
        autonat: autonat_behaviour,
        dcutr: dcutr_behaviour,
        relay_client: relay_client_behaviour,
        relay_server: relay_server_behaviour,
        upnp: upnp_behaviour,
    };

    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(local_key.clone())
        .with_tokio()
        .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default)?
        .with_quic() 
        .with_dns()?
        .with_behaviour(|_| behaviour)?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    let static_port = 4001;
    let _ = swarm.listen_on(format!("/ip4/0.0.0.0/udp/{}/quic-v1", static_port).parse()?);
    let _ = swarm.listen_on(format!("/ip4/0.0.0.0/tcp/{}", static_port).parse()?);

    for node in BOOTSTRAP_NODES {
        if let Ok(addr) = node.parse::<Multiaddr>() {
            let _ = swarm.dial(addr.clone());
        }
    }
    let _ = swarm.behaviour_mut().kademlia.bootstrap();

    let rendezvous_key = kad::RecordKey::new(&b"project-swarm-rendezvous-v1");

    let mut listen_addrs: Vec<Multiaddr> = Vec::new();
    let mut pending_dials: HashSet<PeerId> = HashSet::new();
    let mut is_providing = false;
    let mut known_providers: HashSet<PeerId> = HashSet::new();

    loop {
        tokio::select! {
            // Receive commands from the GUI
            Some(cmd) = rx.recv() => {
                match cmd {
                    crate::NetworkCommand::SendMessage(input) => {
                        let db_clone = Arc::clone(&db);
                        let local_author_clone = local_author_id.clone();
                        let topic_to_publish = gossipsub::IdentTopic::new(current_topic.clone());
                        let app_handle_clone = app.clone();
                        
                        let parents_result = tokio::task::spawn_blocking(move || -> Result<store::DagMessage, String> {
                            let lock = db_clone.lock().map_err(|_| "Database lock failed")?;
                            let p = lock.get_latest_leaves().unwrap_or_default();
                            let dag_msg = store::DagMessage::new(local_author_clone, p, input);
                            
                            lock.save_message(&dag_msg).map_err(|e| format!("DB Write Error: {}", e))?;
                            Ok(dag_msg)
                        }).await;

                        match parents_result {
                            Ok(Ok(parents)) => {
                                match serde_json::to_vec(&parents) {
                                    Ok(payload) => {
                                        let sync_status = match swarm.behaviour_mut().gossipsub.publish(topic_to_publish, payload) {
                                            Ok(_) => "Sent",
                                            Err(_) => "Pending Sync",
                                        };

                                        let _ = app.emit("message-sent", serde_json::json!({
                                            "hash": format!("{} ({})", &parents.id[..8], sync_status),
                                            "text": parents.content,
                                            "sender": "Me",
                                            "isSelf": true
                                        }));
                                    }
                                    Err(e) => {
                                        let _ = app.emit("network-status", format!("🔴 Serialization failed: {}", e));
                                    }
                                }
                            }
                            Ok(Err(db_err)) => {
                                let _ = app.emit("network-status", format!("🔴 {}", db_err));
                            }
                            Err(join_err) => {
                                let _ = app.emit("network-status", format!("🔴 Thread panic: {}", join_err));
                            }
                        }
                    }

                    crate::NetworkCommand::DiscoverPeers => {
                        let _ = app.emit("network-status", "🔍 Querying Global DHT for public nodes...".to_string());
                        swarm.behaviour_mut().kademlia.get_providers(rendezvous_key.clone());
                    }
                    
                    // [NEW] Handle ML-KEM Whispers
                    crate::NetworkCommand::Whisper { peer_id, message } => {
                        if let Ok(target_peer) = peer_id.parse::<PeerId>() {
                            let db_clone = Arc::clone(&db);
                            let target_str = target_peer.to_string();
                            let keys_result = tokio::task::spawn_blocking(move || {
                                db_clone.lock().map_err(|_| "DB Lock failed".to_string())
                                    .and_then(|lock| lock.get_peer_keys(&target_str).map_err(|e| e.to_string()))
                            }).await;

                            match keys_result {
                                Ok(Ok(Some((x25519_pub, mlkem_pub)))) => {
                                    if let Ok(bundle) = crypto::seal_for_network(
                                        message.as_bytes(),
                                        &x25519_pub,
                                        &mlkem_pub
                                    ) {
                                        let payload = serde_json::to_vec(&bundle).unwrap();
                                        let topic_to_publish = gossipsub::IdentTopic::new(current_topic.clone());
                                        let _ = swarm.behaviour_mut().gossipsub.publish(topic_to_publish, payload);
                                        let _ = app.emit("network-status", format!("🟢 Whisper sealed and sent to {}", &peer_id[..8]));
                                        
                                        // Reflect it in our own UI
                                        let _ = app.emit("message-sent", serde_json::json!({
                                            "hash": "WHISPER",
                                            "text": format!("To [{}]: {}", &peer_id[..8], message),
                                            "sender": "Me",
                                            "isSelf": true
                                        }));
                                    } else {
                                        let _ = app.emit("network-status", "🔴 Failed to encrypt whisper.".to_string());
                                    }
                                }
                                _ => {
                                    let _ = app.emit("network-status", format!("🔴 Keys for {} not found. DHT must connect first.", &peer_id[..8]));
                                }
                            }
                        } else {
                            let _ = app.emit("network-status", "🔴 Invalid PeerId format.".to_string());
                        }
                    }

                    crate::NetworkCommand::GenerateInvite => {
                        let mut rng_bytes = [0u8; 4];
                        rand_core::OsRng.fill_bytes(&mut rng_bytes);
                        let room_code = hex::encode(rng_bytes);
                        let invite_hash = format!("swarm-room-{}", room_code);
                        
                        let old_topic = gossipsub::IdentTopic::new(current_topic.clone());
                        let _ = swarm.behaviour_mut().gossipsub.unsubscribe(&old_topic);
                        
                        current_topic = invite_hash.clone();
                        let new_topic = gossipsub::IdentTopic::new(current_topic.clone());
                        let _ = swarm.behaviour_mut().gossipsub.subscribe(&new_topic);
                        
                        let room_key = kad::RecordKey::new(&current_topic);
                        let _ = swarm.behaviour_mut().kademlia.start_providing(room_key.into());
                        
                        let mut raw_addrs = Vec::new();
                        for ext in swarm.external_addresses() {
                            raw_addrs.push(ext.to_string());
                        }
                        if raw_addrs.is_empty() {
                            for local in swarm.listeners() {
                                raw_addrs.push(local.to_string());
                            }
                        }

                        let mut final_addrs = Vec::new();
                        for a in raw_addrs {
                            let ma: Multiaddr = a.parse().unwrap();
                            if !ma.to_string().contains("/p2p/") {
                                final_addrs.push(format!("{}/p2p/{}", ma, local_peer_id));
                            } else {
                                final_addrs.push(ma.to_string());
                            }
                        }

                        let invite_data = FatInvite {
                            sender_x25519_pub: my_crypto_id.x25519_public.to_bytes().to_vec(),
                            sender_mlkem_pub: my_crypto_id.mlkem_public.as_bytes().to_vec(),
                            signature: Vec::new(),
                            addrs: final_addrs,
                            topic: current_topic.clone(),
                        };

                        let json = serde_json::to_string(&invite_data).unwrap();
                        let payload_to_sign = json.as_bytes();
                        let signature = local_key.sign(payload_to_sign).unwrap();

                        let mut signed_invite = invite_data;
                        signed_invite.signature = signature.to_vec();
                        
                        let json_signed = serde_json::to_string(&signed_invite).unwrap();
                        let b64_invite = URL_SAFE.encode(json_signed);

                        let _ = app.emit("room-changed", current_topic.clone());
                        let _ = app.emit("invite-generated", b64_invite.clone());
                        let _ = app.emit("network-status", format!("🟡 Generated signed Fat Invite for private room."));
                    }
                    crate::NetworkCommand::JoinRoom(target_b64) => {
                        match URL_SAFE.decode(target_b64.trim()) {
                            Ok(json_bytes) => {
                                if let Ok(invite_data) = serde_json::from_slice::<FatInvite>(&json_bytes) {
                                    let mut invite_copy = invite_data.clone();
                                    invite_copy.signature = Vec::new();
                                    let payload = serde_json::to_string(&invite_copy).unwrap();
                                    
                                    let x_bytes: [u8; 32] = match invite_data.sender_x25519_pub.clone().try_into() {
                                        Ok(b) => b,
                                        Err(_) => {
                                            let _ = app.emit("network-status", "🔴 Invalid sender key in invite.".to_string());
                                            continue;
                                        }
                                    };
                                    let sender_pub_key = libp2p::identity::PublicKey::try_decode_protobuf(&invite_data.sender_x25519_pub)
                                        .unwrap_or_else(|_| libp2p::identity::PublicKey::Ed25519(
                                            libp2p::identity::ed25519::PublicKey::from_slice(&x_bytes)
                                        ));
                                    
                                    let sig_bytes: &[u8] = &invite_data.signature;
                                    if sig_bytes.len() != 64 || !sender_pub_key.verify(payload.as_bytes(), sig_bytes) {
                                        let _ = app.emit("network-status", "🔴 Invalid invite signature. Cannot verify sender identity.".to_string());
                                        continue;
                                    }
                                    
                                    let _ = app.emit("network-status", format!("Joining room: '{}'", invite_data.topic));
                                    
                                    let old_topic = gossipsub::IdentTopic::new(current_topic.clone());
                                    let _ = swarm.behaviour_mut().gossipsub.unsubscribe(&old_topic);
                                    
                                    current_topic = invite_data.topic.clone();
                                    let new_topic = gossipsub::IdentTopic::new(current_topic.clone());
                                    let _ = swarm.behaviour_mut().gossipsub.subscribe(&new_topic);
                                    
                                    let room_key = kad::RecordKey::new(&current_topic);
                                    let _ = swarm.behaviour_mut().kademlia.start_providing(room_key.clone().into());
                                    
                                    for addr_str in invite_data.addrs {
                                        if let Ok(addr) = addr_str.parse::<Multiaddr>() {
                                            let _ = swarm.dial(addr);
                                        }
                                    }
                                    swarm.behaviour_mut().kademlia.get_providers(room_key);
                                    
                                    let _ = app.emit("room-changed", current_topic.clone());
                                } else {
                                    let _ = app.emit("network-status", "🔴 Invalid or corrupted invite format.".to_string());
                                }
                            }
                            Err(_) => {
                                let _ = app.emit("network-status", "🔴 Invalid Base64 invite string.".to_string());
                            }
                        }
                    }
                }
            }
            
            event = swarm.select_next_some() => match event {
                // [NEW] Emergent AutoNAT Promotion Event
                SwarmEvent::Behaviour(SwarmProtocolEvent::Autonat(autonat::Event::StatusChanged { old: _, new })) => {
                    if let autonat::NatStatus::Public(addr) = new {
                        let _ = app.emit("network-status", format!("🌐 Public IP Detected. Promoted to Emergent Relay Server ({})", addr));
                    }
                },

                SwarmEvent::Behaviour(SwarmProtocolEvent::Identify(identify::Event::Received { peer_id, info })) => {
                    let observed_ip = info.observed_addr.clone();
                    if !listen_addrs.contains(&observed_ip) && listen_addrs.len() < 6 {
                        listen_addrs.push(observed_ip.clone());
                        let _ = swarm.add_external_address(observed_ip);
                    }
                    for addr in info.listen_addrs {
                        swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
                    }
                    if !is_providing {
                        let _ = swarm.behaviour_mut().kademlia.start_providing(rendezvous_key.clone().into());
                        is_providing = true;
                    }

                    if info.protocols.contains(&kex::KEX_PROTOCOL_NAME) {
                        let mut payload_to_sign = my_crypto_id.x25519_public.to_bytes().to_vec();
                        payload_to_sign.extend_from_slice(my_crypto_id.mlkem_public.as_bytes());
                        let signature = local_key.sign(&payload_to_sign).unwrap();

                        swarm.behaviour_mut().kex.send_request(
                            &peer_id,
                            kex::KexRequest {
                                x25519_pub: my_crypto_id.x25519_public.to_bytes().to_vec(),
                                mlkem_pub: my_crypto_id.mlkem_public.as_bytes().to_vec(),
                                signature,
                            }
                        );
                    }
                }

                SwarmEvent::Behaviour(SwarmProtocolEvent::Kademlia(kad::Event::OutboundQueryProgressed { 
                    result: kad::QueryResult::GetProviders(Ok(kad::GetProvidersOk::FoundProviders { providers, .. })), .. 
                })) => {
                    for provider in providers {
                        if provider != local_peer_id && !swarm.is_connected(&provider) {
                            if known_providers.insert(provider) {
                                let _ = swarm.dial(provider);
                                pending_dials.insert(provider);
                            }
                        }
                    }
                }

                SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                    match endpoint {
                        libp2p::core::ConnectedPoint::Dialer { address, .. } => {
                            swarm.behaviour_mut().kademlia.add_address(&peer_id, address.clone());
                        }
                        libp2p::core::ConnectedPoint::Listener { send_back_addr, .. } => {
                            swarm.behaviour_mut().kademlia.add_address(&peer_id, send_back_addr.clone());
                        }
                    }
                    swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                }

                SwarmEvent::Behaviour(SwarmProtocolEvent::Kex(request_response::Event::Message { peer, message })) => match message {
                    request_response::Message::Request { request, channel, .. } => {
                        let mut verify_payload = request.x25519_pub.clone();
                        verify_payload.extend_from_slice(&request.mlkem_pub);
                        let pub_key = libp2p::identity::PublicKey::try_decode_protobuf(&peer.to_bytes()[2..]).unwrap();
                        
                        if pub_key.verify(&verify_payload, &request.signature) {
                            // [NEW] Save keys to DB so we can whisper them later!
                            let db_clone = Arc::clone(&db);
                            let peer_str = peer.to_string();
                            let req_clone = request.clone();
                            let _ = tokio::task::spawn_blocking(move || {
                                let _ = db_clone.lock().unwrap().save_peer_keys(&peer_str, &req_clone.x25519_pub, &req_clone.mlkem_pub, &req_clone.signature);
                            }).await;

                            let mut payload_to_sign = my_crypto_id.x25519_public.to_bytes().to_vec();
                            payload_to_sign.extend_from_slice(my_crypto_id.mlkem_public.as_bytes());
                            let signature = local_key.sign(&payload_to_sign).unwrap();
                            let _ = swarm.behaviour_mut().kex.send_response(channel, kex::KexResponse {
                                x25519_pub: my_crypto_id.x25519_public.to_bytes().to_vec(),
                                mlkem_pub: my_crypto_id.mlkem_public.as_bytes().to_vec(),
                                signature,
                            });
                            let _ = app.emit("network-status", format!("🟢 Secure link established with {}", &peer.to_string()[..8]));
                        }
                    }
                    request_response::Message::Response { response, .. } => {
                        let mut verify_payload = response.x25519_pub.clone();
                        verify_payload.extend_from_slice(&response.mlkem_pub);
                        let pub_key = libp2p::identity::PublicKey::try_decode_protobuf(&peer.to_bytes()[2..]).unwrap();
                        
                        if pub_key.verify(&verify_payload, &response.signature) {
                            // [NEW] Save keys to DB
                            let db_clone = Arc::clone(&db);
                            let peer_str = peer.to_string();
                            let _ = tokio::task::spawn_blocking(move || {
                                let _ = db_clone.lock().unwrap().save_peer_keys(&peer_str, &response.x25519_pub, &response.mlkem_pub, &response.signature);
                            }).await;

                            let _ = app.emit("network-status", format!("🟢 Secure link established with {}", &peer.to_string()[..8]));
                        }
                    }
                }

                SwarmEvent::Behaviour(SwarmProtocolEvent::Gossipsub(gossipsub::Event::Message { message, .. })) => {
                    // [NEW] Attempt to decrypt as an ML-KEM Whisper first
                    if let Ok(bundle) = serde_json::from_slice::<crypto::EncryptedBundle>(&message.data) {
                        if let Ok(decrypted) = crypto::open_payload(&bundle, &my_crypto_id) {
                            let text = String::from_utf8_lossy(&decrypted);
                            let sender = message.source.map(|p| p.to_string()).unwrap_or_else(|| "Unknown".to_string());
                            
                            let _ = app.emit("incoming-message", serde_json::json!({
                                "hash": "WHISPER",
                                "text": text.to_string(),
                                "sender": format!("{} [WHISPER]", &sender[..8]),
                                "isSelf": false
                            }));
                        }
                    } 
                    // Fallback to standard DAG message parsing
                    else if let Ok(dag_msg) = serde_json::from_slice::<store::DagMessage>(&message.data) {
                        let sender_short = message.source.map(|p| p.to_string()).unwrap_or_else(|| "Unknown".to_string());
                        let _ = app.emit("incoming-message", serde_json::json!({
                            "hash": &dag_msg.id[..8],
                            "text": &dag_msg.content,
                            "sender": &sender_short[..8],
                            "isSelf": false
                        }));
                        
                        let db_clone = Arc::clone(&db);
                        let _ = tokio::task::spawn_blocking(move || {
                            let _ = db_clone.lock().unwrap().save_message(&dag_msg);
                        }).await;
                    }
                }
                _ => {}
            }
        }
    }
}