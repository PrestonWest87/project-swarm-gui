<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

  type Message = { sender: string, text: string, hash: string, isSelf: boolean };
  let messages: Message[] = [];
  let currentMessage = "";
  let roomHash = "swarm-alpha";
  let targetJoinHash = "";
  let networkStatus = "🟢 Booting Network Engine...";
  
  // Local Identity
  let username = "username";

  onMount(async () => {
    networkStatus = "🟢 Secure (Listening via DHT)";

    // Listen for self-sent messages returning from Rust
    await listen<Message>("message-sent", (event) => {
      let displayMsg = event.payload;
      try {
        // Unwrap the JSON payload
        let parsed = JSON.parse(displayMsg.text);
        if (parsed.m) displayMsg.text = parsed.m;
      } catch(e) {} // Fallback to raw text if parsing fails
      
      messages = [...messages, displayMsg];
    });

    // Listen for peer messages arriving from the mesh network
    await listen<Message>("incoming-message", (event) => {
      let displayMsg = event.payload;
      try {
        // Unwrap the JSON payload and extract the sender's alias
        let parsed = JSON.parse(displayMsg.text);
        if (parsed.u && parsed.m) {
          // Attach their Cryptographic ID to their chosen alias to prevent spoofing
          displayMsg.sender = `${parsed.u} [${displayMsg.sender}]`;
          displayMsg.text = parsed.m;
        }
      } catch(e) {} 
      
      messages = [...messages, displayMsg];
    });

    await listen<string>("room-changed", (event) => {
      roomHash = event.payload;
      messages = []; // Clear chat on room switch
    });

    await listen<string>("network-status", (event) => {
      networkStatus = `⚡ ${event.payload}`;
      setTimeout(() => { networkStatus = "🟢 Secure (Connected)"; }, 3000);
    });
  });

  async function sendMessage() {
    if (!currentMessage.trim() || !username.trim()) return;
    
    // Wrap the message and identity into a JSON payload for the DAG
    const payload = JSON.stringify({
      u: username,
      m: currentMessage
    });
    
    await invoke("send_message", { message: payload });
    currentMessage = "";
  }

  async function requestInvite() {
    await invoke("generate_invite");
  }

  async function handleJoin() {
    if (!targetJoinHash.trim()) return;
    await invoke("join_room", { hash: targetJoinHash });
    targetJoinHash = "";
  }
</script>

<main class="container">
  <div class="sidebar">
    <div class="header">
      <h2>PROJECT SWARM</h2>
      <span class="status">{networkStatus}</span>
    </div>

    <div class="identity-section">
      <p>Display Name:</p>
      <input type="text" class="alias-input" bind:value={username} placeholder="Set alias..." />
    </div>
    
    <div class="room-info">
      <p>Current Room:</p>
      <div class="hash-box">{roomHash}</div>
      <button class="action-btn" on:click={requestInvite}>Generate Secure Room</button>
      
      <div class="join-section">
        <input type="text" bind:value={targetJoinHash} placeholder="Enter Room Hash..." />
        <button class="action-btn secondary" on:click={handleJoin}>Join Room</button>
      </div>
    </div>
  </div>

  <div class="chat-area">
    <div class="messages">
      {#if messages.length === 0}
        <div class="empty-state">No messages in the local DAG.</div>
      {/if}
      
      {#each messages as msg}
        <div class="message {msg.isSelf ? 'self' : 'peer'}">
          <span class="sender">{msg.sender}</span>
          <p>{msg.text}</p>
          <span class="hash">{msg.hash}</span>
        </div>
      {/each}
    </div>

    <div class="input-area">
      <form on:submit|preventDefault={sendMessage}>
        <input 
          type="text" 
          bind:value={currentMessage} 
          placeholder="Broadcast to swarm..." 
          autocomplete="off"
        />
        <button type="submit">SEND</button>
      </form>
    </div>
  </div>
</main>

<style>
  :global(body) {
    margin: 0;
    padding: 0;
    font-family: 'Segoe UI', Tahoma, Geneva, Verdana, sans-serif;
    background-color: #0f172a;
    color: #e2e8f0;
    height: 100vh;
    overflow: hidden;
  }
  .container { display: flex; height: 100vh; }
  .sidebar {
    width: 280px;
    background-color: #1e293b;
    border-right: 1px solid #334155;
    padding: 20px;
    display: flex;
    flex-direction: column;
  }
  .header h2 { margin: 0 0 5px 0; font-size: 1.2rem; letter-spacing: 2px; color: #38bdf8; }
  .status { font-size: 0.8rem; color: #4ade80; }
  
  .identity-section { margin-top: 30px; }
  .identity-section p { margin: 0 0 5px 0; font-size: 0.85rem; color: #94a3b8; }
  .alias-input {
    width: 90%; padding: 8px 10px; background-color: #0f172a; 
    border: 1px solid #38bdf8; border-radius: 4px; color: #38bdf8; 
    font-weight: bold; font-size: 0.9rem; outline: none;
  }

  .room-info { margin-top: 30px; border-top: 1px solid #334155; padding-top: 20px;}
  .room-info p { margin: 0 0 5px 0; font-size: 0.85rem; color: #94a3b8; }
  .hash-box {
    background-color: #0f172a; padding: 10px; border-radius: 4px; font-family: monospace;
    font-size: 0.8rem; color: #cbd5e1; margin-bottom: 15px; word-break: break-all; border: 1px solid #334155;
  }
  .action-btn {
    width: 100%; padding: 10px; background-color: #0284c7; color: white;
    border: none; border-radius: 4px; cursor: pointer; font-weight: bold; margin-bottom: 10px;
  }
  .action-btn:hover { background-color: #0369a1; }
  .action-btn.secondary { background-color: #334155; margin-top: 5px;}
  .action-btn.secondary:hover { background-color: #475569; }
  .join-section input { width: 90%; padding: 10px; margin-top: 20px; border-radius: 4px; background: #0f172a; border: 1px solid #334155; color: white;}
  .chat-area { flex: 1; display: flex; flex-direction: column; }
  .messages { flex: 1; padding: 20px; overflow-y: auto; display: flex; flex-direction: column; gap: 15px; }
  .empty-state { margin: auto; color: #64748b; font-style: italic; }
  .message { max-width: 70%; padding: 12px 16px; border-radius: 8px; position: relative; }
  .message.self { align-self: flex-end; background-color: #0284c7; border-bottom-right-radius: 0; }
  .message.peer { align-self: flex-start; background-color: #334155; border-bottom-left-radius: 0; }
  .message p { margin: 0; line-height: 1.4; }
  .sender { display: block; font-size: 0.7rem; font-weight: bold; margin-bottom: 4px; color: #cbd5e1; }
  .hash { display: block; font-size: 0.6rem; text-align: right; margin-top: 6px; opacity: 0.6; font-family: monospace; }
  .input-area { padding: 20px; background-color: #1e293b; border-top: 1px solid #334155; }
  form { display: flex; gap: 10px; }
  .input-area input { flex: 1; padding: 12px 16px; background-color: #0f172a; border: 1px solid #334155; border-radius: 4px; color: white; font-size: 1rem; outline: none; }
  .input-area input:focus { border-color: #38bdf8; }
  .input-area button { padding: 0 24px; background-color: #38bdf8; color: #0f172a; border: none; border-radius: 4px; font-weight: bold; cursor: pointer; }
</style>