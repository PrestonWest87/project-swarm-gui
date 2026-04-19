<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount, afterUpdate } from "svelte";

  type Message = { 
    sender: string, 
    text: string, 
    hash: string, 
    isSelf: boolean, 
    isSystem?: boolean,
    isInvite?: boolean,
    inviteCode?: string
  };
  
  let messages: Message[] = [];
  let currentMessage = "";
  let roomHash = "swarm-alpha";
  let networkStatus = "Booting Engine...";
  let isConnected = false;
  let chatContainer: HTMLDivElement;
  
  let username = "Anon";

  // Auto-scroll to bottom on new messages
  afterUpdate(() => {
    if (chatContainer) chatContainer.scrollTop = chatContainer.scrollHeight;
  });

  onMount(async () => {
    networkStatus = "Listening via DHT";
    
    // Display initial welcome and commands
    messages = [...messages, {
      sender: "SYSTEM",
      text: "Welcome to Project Swarm. Transport is End-to-End Encrypted.\nType /help to see available commands.",
      hash: "INIT",
      isSelf: false,
      isSystem: true
    }];

    await listen<Message>("message-sent", (event) => {
      let displayMsg = event.payload;
      try {
        let parsed = JSON.parse(displayMsg.text);
        if (parsed.m) displayMsg.text = parsed.m;
      } catch(e) {} 
      messages = [...messages, displayMsg];
    });

    await listen<Message>("incoming-message", (event) => {
      let displayMsg = event.payload;
      try {
        let parsed = JSON.parse(displayMsg.text);
        if (parsed.u && parsed.m) {
          displayMsg.sender = `${parsed.u} [${displayMsg.sender}]`;
          displayMsg.text = parsed.m;
        }
      } catch(e) {} 
      messages = [...messages, displayMsg];
    });

    await listen<string>("room-changed", (event) => {
      roomHash = event.payload;
      messages = [...messages, {
        sender: "SYSTEM",
        text: `Moved to secure room: ${roomHash}`,
        hash: "",
        isSelf: false,
        isSystem: true
      }];
    });

    await listen<string>("invite-generated", (event) => {
      // Push the base64 invite directly into the chat feed as an interactive element
      messages = [...messages, {
        sender: "SYSTEM",
        text: "Secure relay bridge generated. Share this string with your peer:",
        hash: "",
        isSelf: false,
        isSystem: true,
        isInvite: true,
        inviteCode: event.payload
      }];
    });

    await listen<string>("network-status", (event) => {
      networkStatus = event.payload;
      isConnected = true;
    });
  });

  async function handleInput() {
    if (!currentMessage.trim()) return;
    const input = currentMessage.trim();
    currentMessage = ""; // Clear input immediately

    // --- COMMAND INTERCEPTION ---
    if (input === "/help") {
      messages = [...messages, {
        sender: "SYSTEM",
        text: "AVAILABLE COMMANDS:\n/help - Show this list\n/discover - Query the global DHT for public nodes\n/invite - Generate a direct-connect bridge\n/join <base64_string> - Connect to a peer's room\n/whisper <PeerId> <message> - Send a post-quantum encrypted DM",
        hash: "",
        isSelf: false,
        isSystem: true
      }];
      return;
    }

    if (input === "/discover") {
      messages = [...messages, { sender: "SYSTEM", text: "Querying Global DHT for public nodes...", hash: "", isSelf: false, isSystem: true }];
      await invoke("discover_peers");
      return;
    }

    if (input === "/invite") {
      await invoke("generate_invite");
      return;
    }

    if (input.startsWith("/join ")) {
      const targetJoinHash = input.substring(6).trim();
      if (targetJoinHash) {
        messages = [...messages, { sender: "SYSTEM", text: "Authenticating invite and negotiating connection...", hash: "", isSelf: false, isSystem: true }];
        await invoke("join_room", { hash: targetJoinHash });
      }
      return;
    }

    if (input.startsWith("/whisper ")) {
      const parts = input.split(" ");
      if (parts.length >= 3) {
        const targetId = parts[1];
        const whisperText = parts.slice(2).join(" ");
        await invoke("whisper_peer", { peer_id: targetId, message: whisperText });
      } else {
        messages = [...messages, { sender: "SYSTEM", text: "Usage: /whisper <PeerId> <message>", hash: "", isSelf: false, isSystem: true }];
      }
      return;
    }

    // --- NORMAL MESSAGING ---
    if (!username.trim()) username = "Anon";
    
    const payload = JSON.stringify({
      u: username,
      m: input
    });
    
    await invoke("send_message", { message: payload });
  }

  function copyInvite(code: string) {
    navigator.clipboard.writeText(code);
    messages = [...messages, {
      sender: "SYSTEM",
      text: "Invite copied to clipboard!",
      hash: "",
      isSelf: false,
      isSystem: true
    }];
  }
</script>

<main class="app-layout">
  <aside class="sidebar">
    <div class="brand">
      <div class="brand-icon"></div>
      <h1>SWARM</h1>
    </div>

    <div class="status-panel">
      <div class="status-indicator {isConnected ? 'connected' : 'booting'}"></div>
      <span class="status-text">{networkStatus}</span>
    </div>

    <div class="settings-group">
      <label for="alias">CRYPTOGRAPHIC ALIAS</label>
      <input id="alias" type="text" bind:value={username} placeholder="Enter display name..." />
    </div>

    <div class="settings-group">
      <label>CURRENT TOPIC</label>
      <div class="room-badge">{roomHash.substring(0, 16)}...</div>
    </div>
    
    <div class="quick-actions">
      <p class="section-title">QUICK ACTIONS</p>
      <button class="btn primary" on:click={() => { currentMessage = "/invite"; handleInput(); }}>Generate Invite</button>
      <p class="hint">Or type <code>/help</code> in chat</p>
    </div>
  </aside>

  <section class="chat-interface">
    <div class="chat-header">
      <h2>Mesh Network Chatter</h2>
      <div class="header-tools">E2E Encrypted</div>
    </div>

    <div class="messages-container" bind:this={chatContainer}>
      {#each messages as msg}
        {#if msg.isSystem}
          <div class="system-message">
            <span class="sys-icon">⚡</span>
            <div class="sys-content">
              <p>{msg.text}</p>
              {#if msg.isInvite && msg.inviteCode}
                <div class="invite-box">
                  <input type="text" readonly value={msg.inviteCode} />
                  <button on:click={() => copyInvite(msg.inviteCode || '')}>Copy</button>
                </div>
              {/if}
            </div>
          </div>
        {:else}
          <div class="message-wrapper {msg.isSelf ? 'self' : 'peer'}">
            <div class="message-bubble">
              <div class="message-meta">
                <span class="sender">{msg.sender}</span>
                <span class="hash">{msg.hash}</span>
              </div>
              <p class="message-text">{msg.text}</p>
            </div>
          </div>
        {/if}
      {/each}
    </div>

    <div class="input-container">
      <form on:submit|preventDefault={handleInput}>
        <input 
          type="text" 
          bind:value={currentMessage} 
          placeholder="Message swarm or type /help..." 
          autocomplete="off"
          autofocus
        />
        <button type="submit">
          <svg viewBox="0 0 24 24" width="20" height="20" stroke="currentColor" stroke-width="2" fill="none" stroke-linecap="round" stroke-linejoin="round"><line x1="22" y1="2" x2="11" y2="13"></line><polygon points="22 2 15 22 11 13 2 9 22 2"></polygon></svg>
        </button>
      </form>
    </div>
  </section>
</main>

<style>
  :global(body) {
    margin: 0; padding: 0;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
    background-color: #0f172a;
    color: #f8fafc;
    height: 100vh;
    overflow: hidden;
  }

  .app-layout { display: flex; height: 100vh; }

  /* SIDEBAR STYLING */
  .sidebar {
    width: 260px;
    background-color: #1e293b;
    border-right: 1px solid #334155;
    padding: 24px 20px;
    display: flex;
    flex-direction: column;
    gap: 24px;
  }

  .brand { display: flex; align-items: center; gap: 12px; }
  .brand-icon { width: 14px; height: 14px; background: #38bdf8; border-radius: 50%; box-shadow: 0 0 10px #38bdf8; }
  .brand h1 { margin: 0; font-size: 1.25rem; font-weight: 800; letter-spacing: 2px; color: #f8fafc; }

  .status-panel { display: flex; align-items: center; gap: 10px; background: #0f172a; padding: 10px 14px; border-radius: 8px; border: 1px solid #334155; }
  .status-indicator { width: 8px; height: 8px; border-radius: 50%; }
  .status-indicator.booting { background: #f59e0b; }
  .status-indicator.connected { background: #10b981; box-shadow: 0 0 8px #10b981; }
  .status-text { font-size: 0.8rem; color: #cbd5e1; font-weight: 500; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;}

  .settings-group { display: flex; flex-direction: column; gap: 8px; }
  .settings-group label { font-size: 0.7rem; font-weight: 700; color: #64748b; letter-spacing: 1px; }
  .settings-group input { 
    background: #0f172a; border: 1px solid #334155; color: #38bdf8;
    padding: 10px 12px; border-radius: 6px; font-weight: 600; font-size: 0.9rem; outline: none; transition: border-color 0.2s;
  }
  .settings-group input:focus { border-color: #38bdf8; }
  
  .room-badge { background: #334155; padding: 8px 12px; border-radius: 6px; font-family: monospace; font-size: 0.85rem; color: #94a3b8; border: 1px solid #475569; }

  .quick-actions { margin-top: auto; }
  .section-title { font-size: 0.7rem; font-weight: 700; color: #64748b; letter-spacing: 1px; margin-bottom: 12px; }
  .btn { width: 100%; padding: 12px; border-radius: 6px; border: none; font-weight: 600; cursor: pointer; transition: all 0.2s; }
  .btn.primary { background: #0284c7; color: white; }
  .btn.primary:hover { background: #0369a1; transform: translateY(-1px); }
  .hint { font-size: 0.75rem; color: #64748b; text-align: center; margin-top: 12px; }
  .hint code { background: #0f172a; padding: 2px 6px; border-radius: 4px; border: 1px solid #334155; }

  /* CHAT AREA STYLING */
  .chat-interface { flex: 1; display: flex; flex-direction: column; background: #0f172a; }
  
  .chat-header { padding: 20px 24px; border-bottom: 1px solid #1e293b; display: flex; justify-content: space-between; align-items: center; }
  .chat-header h2 { margin: 0; font-size: 1.1rem; font-weight: 600; color: #e2e8f0; }
  .header-tools { font-size: 0.75rem; font-weight: 600; color: #10b981; background: rgba(16, 185, 129, 0.1); padding: 4px 10px; border-radius: 12px; border: 1px solid rgba(16, 185, 129, 0.2); }

  .messages-container { flex: 1; overflow-y: auto; padding: 24px; display: flex; flex-direction: column; gap: 20px; }
  
  .message-wrapper { display: flex; flex-direction: column; max-width: 75%; }
  .message-wrapper.self { align-self: flex-end; align-items: flex-end; }
  .message-wrapper.peer { align-self: flex-start; align-items: flex-start; }
  
  .message-bubble { padding: 12px 16px; border-radius: 12px; position: relative; }
  .self .message-bubble { background: #0284c7; border-bottom-right-radius: 4px; }
  .peer .message-bubble { background: #1e293b; border-bottom-left-radius: 4px; border: 1px solid #334155; }
  
  .message-meta { display: flex; justify-content: space-between; gap: 12px; margin-bottom: 6px; align-items: center; }
  .sender { font-size: 0.75rem; font-weight: 700; color: #bae6fd; }
  .peer .sender { color: #94a3b8; }
  .hash { font-size: 0.65rem; font-family: monospace; opacity: 0.5; }
  
  .message-text { margin: 0; font-size: 0.95rem; line-height: 1.5; word-wrap: break-word; white-space: pre-wrap; }

  /* SYSTEM / INVITE MESSAGE STYLING */
  .system-message { display: flex; gap: 12px; align-items: flex-start; margin: 10px 0; max-width: 85%; align-self: center; background: #1e293b; padding: 16px; border-radius: 8px; border-left: 3px solid #38bdf8; }
  .sys-icon { font-size: 1.2rem; }
  .sys-content p { margin: 0; font-size: 0.9rem; color: #cbd5e1; white-space: pre-wrap; line-height: 1.5; }
  
  .invite-box { display: flex; margin-top: 12px; gap: 8px; }
  .invite-box input { flex: 1; background: #0f172a; border: 1px solid #334155; color: #94a3b8; padding: 8px 12px; border-radius: 6px; font-family: monospace; font-size: 0.8rem; outline: none; }
  .invite-box button { background: #38bdf8; color: #0f172a; border: none; border-radius: 6px; padding: 0 16px; font-weight: 700; cursor: pointer; transition: background 0.2s; }
  .invite-box button:hover { background: #7dd3fc; }

  /* INPUT AREA STYLING */
  .input-container { padding: 20px 24px; background: #0f172a; border-top: 1px solid #1e293b; }
  .input-container form { display: flex; gap: 12px; background: #1e293b; padding: 8px; border-radius: 12px; border: 1px solid #334155; transition: border-color 0.2s; }
  .input-container form:focus-within { border-color: #38bdf8; }
  
  .input-container input { flex: 1; background: transparent; border: none; color: #f8fafc; font-size: 1rem; padding: 8px 12px; outline: none; }
  .input-container input::placeholder { color: #64748b; }
  
  .input-container button { display: flex; align-items: center; justify-content: center; background: #38bdf8; color: #0f172a; border: none; border-radius: 8px; width: 44px; height: 44px; cursor: pointer; transition: all 0.2s; }
  .input-container button:hover { background: #7dd3fc; transform: scale(1.05); }
</style>