/**
 * Ergo Node Panel - Dashboard Application
 *
 * Features:
 * - Real-time sync status monitoring via WebSocket
 * - Animated mempool visualization with physics simulation
 * - Fallback to polling when WebSocket unavailable
 */

// === Configuration ===
const CONFIG = {
  API_BASE: "",
  WS_RECONNECT_DELAY: 3000,
  POLL_INTERVAL: 3000,
  TARGET_HEIGHT: 1500000, // Approximate mainnet height for progress calculation
  MAX_DISPLAYED_TXS: 100,
  PHYSICS: {
    GRAVITY: 0.25,
    FRICTION: 0.7,
    BOUNCE: 0.4,
    SETTLE_THRESHOLD: 0.3,
  },
};

// === State ===
const state = {
  fullHeight: 0,
  headersHeight: 0,
  isSynced: false,
  peerCount: 0,
  mempoolCount: 0,
  mempoolSize: 0,
  transactions: [],
  isMining: false,
  nodeName: "",
  wsConnected: false,
  recentBlocks: [],
  previousBlockHeight: 0,
};

// === DOM Elements ===
const elements = {
  version: document.getElementById("version"),
  nodeName: document.getElementById("node-name"),
  connectionBanner: document.getElementById("connection-banner"),
  connectionText: document.getElementById("connection-text"),
  statusIndicator: document.getElementById("status-indicator"),
  syncLabel: document.getElementById("sync-label"),
  miningBadge: document.getElementById("mining-badge"),
  headersProgress: document.getElementById("headers-progress"),
  headersCount: document.getElementById("headers-count"),
  blocksProgress: document.getElementById("blocks-progress"),
  blocksCount: document.getElementById("blocks-count"),
  peerCount: document.getElementById("peer-count"),
  mempoolCount: document.getElementById("mempool-count"),
  mempoolSize: document.getElementById("mempool-size"),
  mempoolSection: document.getElementById("mempool-section"),
  canvas: document.getElementById("mempool-canvas"),
  emptyMempool: document.getElementById("empty-mempool"),
  blocksSection: document.getElementById("blocks-section"),
  blocksContainer: document.getElementById("blocks-container"),
};

// === Mempool Visualizer ===
class MempoolVisualizer {
  constructor(canvas) {
    this.canvas = canvas;
    this.ctx = canvas.getContext("2d");
    this.transactions = new Map(); // id -> tx object
    this.running = false;
    this.resizeObserver = null;

    this.setupCanvas();
  }

  setupCanvas() {
    // Set up resize observer
    this.resizeObserver = new ResizeObserver(() => this.resize());
    this.resizeObserver.observe(this.canvas.parentElement);
    this.resize();
  }

  resize() {
    const container = this.canvas.parentElement;
    const dpr = window.devicePixelRatio || 1;

    this.canvas.width = container.clientWidth * dpr;
    this.canvas.height = container.clientHeight * dpr;
    this.canvas.style.width = container.clientWidth + "px";
    this.canvas.style.height = container.clientHeight + "px";

    this.ctx.scale(dpr, dpr);
    this.width = container.clientWidth;
    this.height = container.clientHeight;
  }

  start() {
    if (this.running) return;
    this.running = true;
    this.animate();
  }

  stop() {
    this.running = false;
  }

  addTransaction(tx) {
    if (this.transactions.has(tx.id)) return;

    // Calculate visual properties - use radius for circles
    const baseRadius = Math.sqrt(tx.size) * 0.75;
    const radius = Math.max(Math.min(baseRadius, 22), 6);
    const feeRate = tx.size > 0 ? tx.fee / tx.size : 0;

    const txObj = {
      id: tx.id,
      x: Math.random() * (this.width - radius * 2),
      y: -radius * 2 - Math.random() * 50,
      radius: radius,
      width: radius * 2, // For collision compat
      height: radius * 2,
      vx: (Math.random() - 0.5) * 2,
      vy: 0,
      color: this.feeToColor(feeRate),
      settled: false,
      removing: false,
      confirming: false, // For downward animation into block
      alpha: 1,
      fee: tx.fee,
      size: tx.size,
    };

    this.transactions.set(tx.id, txObj);
  }

  removeTransaction(txId) {
    const tx = this.transactions.get(txId);
    if (tx && !tx.removing) {
      tx.removing = true;
      tx.vy = -8;
      tx.vr = (Math.random() - 0.5) * 0.1;

      // Remove after animation
      setTimeout(() => {
        this.transactions.delete(txId);
      }, 600);
    }
  }

  feeToColor(feeRate) {
    // Normalize fee rate (0.001 ERG/byte is typical)
    // Below 0.003: low (blue)
    // 0.003 - 0.01: medium (yellow)
    // Above 0.01: high (red)
    const normalized = Math.min(feeRate / 0.015, 1);

    if (normalized < 0.33) {
      // Blue to cyan
      const t = normalized / 0.33;
      return `hsl(${210 - t * 30}, 70%, ${55 + t * 10}%)`;
    } else if (normalized < 0.66) {
      // Cyan to yellow
      const t = (normalized - 0.33) / 0.33;
      return `hsl(${180 - t * 135}, 75%, ${60 + t * 5}%)`;
    } else {
      // Yellow to red
      const t = (normalized - 0.66) / 0.34;
      return `hsl(${45 - t * 45}, 80%, ${55 - t * 10}%)`;
    }
  }

  animate() {
    if (!this.running) return;

    // Clear canvas
    this.ctx.fillStyle = "#0f0f1a";
    this.ctx.fillRect(0, 0, this.width, this.height);

    // Draw ground line
    this.ctx.strokeStyle = "rgba(255, 255, 255, 0.05)";
    this.ctx.lineWidth = 1;
    this.ctx.beginPath();
    this.ctx.moveTo(0, this.height - 5);
    this.ctx.lineTo(this.width, this.height - 5);
    this.ctx.stroke();

    // Sort transactions by y position for proper rendering
    const sortedTxs = Array.from(this.transactions.values()).sort(
      (a, b) => a.y - b.y,
    );

    for (const tx of sortedTxs) {
      this.updateTransaction(tx);
      this.drawTransaction(tx);
    }

    requestAnimationFrame(() => this.animate());
  }

  updateTransaction(tx) {
    const { GRAVITY, FRICTION, BOUNCE, SETTLE_THRESHOLD } = CONFIG.PHYSICS;

    // Handle confirmation animation (moving DOWN into block)
    if (tx.confirming) {
      tx.vy = Math.min(tx.vy + 0.3, 8); // Accelerate down
      tx.y += tx.vy;

      // Fade out near bottom
      if (tx.y > this.height - tx.radius * 4) {
        tx.alpha = Math.max(0, tx.alpha - 0.06);
      }

      // Remove when faded
      if (tx.alpha <= 0) {
        this.transactions.delete(tx.id);
      }
      return;
    }

    if (tx.removing) {
      // Fly up and fade out
      tx.vy -= 0.5;
      tx.y += tx.vy;
      tx.alpha = Math.max(0, tx.alpha - 0.03);
      return;
    }

    if (tx.settled) return;

    // Apply gravity
    tx.vy += GRAVITY;

    // Apply velocity
    tx.x += tx.vx;
    tx.y += tx.vy;

    // Floor collision
    const floor = this.height - tx.height - 5;
    if (tx.y > floor) {
      tx.y = floor;
      tx.vy *= -BOUNCE;
      tx.vx *= FRICTION;

      // Check if settled
      if (Math.abs(tx.vy) < SETTLE_THRESHOLD) {
        tx.settled = true;
        tx.vy = 0;
      }
    }

    // Wall collisions
    if (tx.x < 0) {
      tx.x = 0;
      tx.vx *= -FRICTION;
    } else if (tx.x + tx.width > this.width) {
      tx.x = this.width - tx.width;
      tx.vx *= -FRICTION;
    }

    // Simple collision with other settled transactions
    for (const other of this.transactions.values()) {
      if (other === tx || other.removing || other.confirming) continue;
      if (!other.settled && other.y < tx.y) continue;

      if (this.checkCollision(tx, other)) {
        // Push up - use circle-based positioning
        const txCenterY = tx.y + tx.radius;
        const otherCenterY = other.y + other.radius;
        if (txCenterY < otherCenterY && tx.vy > 0) {
          tx.y = other.y - tx.radius * 2;
          tx.vy *= -BOUNCE * 0.5;

          if (Math.abs(tx.vy) < SETTLE_THRESHOLD) {
            tx.settled = true;
            tx.vy = 0;
          }
        }
      }
    }
  }

  // Animate transaction moving DOWN (into a confirmed block)
  confirmTransaction(txId) {
    const tx = this.transactions.get(txId);
    if (tx && !tx.confirming && !tx.removing) {
      tx.confirming = true;
      tx.settled = false;
      tx.vy = 4; // Initial downward velocity
    }
  }

  checkCollision(a, b) {
    // Circle-based collision detection
    const ax = a.x + a.radius;
    const ay = a.y + a.radius;
    const bx = b.x + b.radius;
    const by = b.y + b.radius;
    const dx = ax - bx;
    const dy = ay - by;
    const distance = Math.sqrt(dx * dx + dy * dy);
    return distance < a.radius + b.radius;
  }

  drawTransaction(tx) {
    this.ctx.save();
    this.ctx.globalAlpha = tx.alpha;

    // Circle center position
    const cx = tx.x + tx.radius;
    const cy = tx.y + tx.radius;

    // Draw circle
    this.ctx.beginPath();
    this.ctx.arc(cx, cy, tx.radius, 0, Math.PI * 2);

    // Radial gradient for 3D effect
    const gradient = this.ctx.createRadialGradient(
      cx - tx.radius * 0.3,
      cy - tx.radius * 0.3,
      0,
      cx,
      cy,
      tx.radius,
    );
    gradient.addColorStop(0, this.lightenColor(tx.color, 15));
    gradient.addColorStop(1, this.darkenColor(tx.color, 15));
    this.ctx.fillStyle = gradient;
    this.ctx.fill();

    // Border - green when confirming, white otherwise
    this.ctx.strokeStyle = tx.confirming
      ? "rgba(74, 222, 128, 0.8)"
      : "rgba(255, 255, 255, 0.2)";
    this.ctx.lineWidth = 1;
    this.ctx.stroke();

    // Highlight on removing (confirmed)
    if (tx.removing) {
      this.ctx.fillStyle = `rgba(74, 222, 128, ${tx.alpha * 0.5})`;
      this.ctx.fill();
    }

    this.ctx.restore();
  }

  lightenColor(hslColor, amount) {
    const match = hslColor.match(/hsl\((\d+),\s*(\d+)%,\s*(\d+)%\)/);
    if (match) {
      const h = parseInt(match[1]);
      const s = parseInt(match[2]);
      const l = Math.min(100, parseInt(match[3]) + amount);
      return `hsl(${h}, ${s}%, ${l}%)`;
    }
    return hslColor;
  }

  darkenColor(hslColor, amount) {
    const match = hslColor.match(/hsl\((\d+),\s*(\d+)%,\s*(\d+)%\)/);
    if (match) {
      const h = parseInt(match[1]);
      const s = parseInt(match[2]);
      const l = Math.max(0, parseInt(match[3]) - amount);
      return `hsl(${h}, ${s}%, ${l}%)`;
    }
    return hslColor;
  }

  updateTransactions(newTxs) {
    const currentIds = new Set(this.transactions.keys());
    const newIds = new Set(newTxs.map((t) => t.id));

    // Add new transactions
    for (const tx of newTxs) {
      if (!currentIds.has(tx.id)) {
        this.addTransaction(tx);
      }
    }

    // Remove transactions not in new list (unless already confirming/removing)
    for (const id of currentIds) {
      if (!newIds.has(id)) {
        const tx = this.transactions.get(id);
        // Skip if already being animated out
        if (tx && !tx.confirming && !tx.removing) {
          this.removeTransaction(id);
        }
      }
    }

    // Update empty state - count only active transactions
    const activeTxCount = Array.from(this.transactions.values()).filter(
      (tx) => !tx.removing && !tx.confirming,
    ).length;
    elements.emptyMempool.classList.toggle("hidden", activeTxCount > 0);
  }

  clear() {
    this.transactions.clear();
  }
}

// === Block Visualizer ===
class BlockVisualizer {
  constructor(containerId) {
    this.container = document.getElementById(containerId);
    this.blocks = new Map(); // blockId -> block data
  }

  updateBlocks(recentBlocks, isSynced) {
    const section = document.getElementById("blocks-section");

    // Hide section if not synced or no blocks
    if (!isSynced || recentBlocks.length === 0) {
      section.classList.add("hidden");
      return;
    }

    section.classList.remove("hidden");

    // Track existing blocks
    const existingIds = new Set(this.blocks.keys());
    const newBlockIds = new Set(recentBlocks.map((b) => b.id));

    // Add new blocks
    for (const block of recentBlocks) {
      if (!existingIds.has(block.id)) {
        this.addBlock(block, block === recentBlocks[0]);
      } else {
        // Update newest status
        const blockData = this.blocks.get(block.id);
        if (blockData && blockData.element) {
          blockData.element.classList.toggle(
            "newest",
            block === recentBlocks[0],
          );
        }
      }
    }

    // Remove old blocks (keep only those in recentBlocks)
    for (const id of existingIds) {
      if (!newBlockIds.has(id)) {
        this.removeBlock(id);
      }
    }
  }

  addBlock(block, isNewest) {
    const div = document.createElement("div");
    div.className = `block-container${isNewest ? " newest entering" : ""}`;
    div.id = `block-${block.id}`;
    div.innerHTML = `
      <div class="block-header">
        <span class="block-height">#${block.height.toLocaleString()}</span>
        <span class="block-tx-count">${block.transactions.length} txs</span>
      </div>
      <canvas class="block-canvas"></canvas>
    `;

    // Insert at top of container
    if (this.container.firstChild) {
      this.container.insertBefore(div, this.container.firstChild);
    } else {
      this.container.appendChild(div);
    }

    // Store block data
    this.blocks.set(block.id, {
      height: block.height,
      element: div,
      txCount: block.transactions.length,
    });

    // Initialize and draw canvas
    const canvas = div.querySelector("canvas");
    this.drawBlockTxs(canvas, block.transactions.length);

    // Remove 'entering' class after animation
    setTimeout(() => {
      div.classList.remove("entering");
    }, 400);
  }

  removeBlock(blockId) {
    const blockData = this.blocks.get(blockId);
    if (blockData && blockData.element) {
      blockData.element.classList.add("exiting");
      setTimeout(() => {
        blockData.element.remove();
        this.blocks.delete(blockId);
      }, 400);
    }
  }

  drawBlockTxs(canvas, count) {
    const ctx = canvas.getContext("2d");
    const dpr = window.devicePixelRatio || 1;
    const width = canvas.offsetWidth || 300;
    const height = 120;

    canvas.width = width * dpr;
    canvas.height = height * dpr;
    canvas.style.width = width + "px";
    canvas.style.height = height + "px";
    ctx.scale(dpr, dpr);

    // Background
    ctx.fillStyle = "#0a0a14";
    ctx.fillRect(0, 0, width, height);

    // Draw ground line
    ctx.strokeStyle = "rgba(255, 255, 255, 0.05)";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(0, height - 2);
    ctx.lineTo(width, height - 2);
    ctx.stroke();

    // Draw circles matching mempool style - larger and with varied colors
    const radius = 12;
    const padding = 6;
    const cols = Math.floor(width / (radius * 2 + padding));

    for (let i = 0; i < Math.min(count, 100); i++) {
      const col = i % cols;
      const row = Math.floor(i / cols);
      const x = padding + col * (radius * 2 + padding) + radius;
      const y = height - 8 - radius - row * (radius * 2 + padding);

      // Skip if would draw above canvas
      if (y - radius < 0) continue;

      // Use varied colors similar to mempool (confirmed = greenish tint)
      const hue = 140 + (i % 5) * 8; // Vary hue slightly for visual interest
      const saturation = 50 + (i % 3) * 10;
      const lightness = 45;

      // Draw circle with gradient
      ctx.beginPath();
      ctx.arc(x, y, radius, 0, Math.PI * 2);

      const gradient = ctx.createRadialGradient(
        x - radius * 0.3,
        y - radius * 0.3,
        0,
        x,
        y,
        radius,
      );
      gradient.addColorStop(
        0,
        `hsl(${hue}, ${saturation}%, ${lightness + 15}%)`,
      );
      gradient.addColorStop(
        1,
        `hsl(${hue}, ${saturation}%, ${lightness - 10}%)`,
      );
      ctx.fillStyle = gradient;
      ctx.fill();

      ctx.strokeStyle = "rgba(255, 255, 255, 0.2)";
      ctx.lineWidth = 1;
      ctx.stroke();
    }
  }
}

// === Global visualizer instances ===
let visualizer = null;
let blockVisualizer = null;

// === API Functions ===
async function fetchInfo() {
  try {
    const response = await fetch(`${CONFIG.API_BASE}/info`);
    if (!response.ok) throw new Error("API error");
    return await response.json();
  } catch (e) {
    console.error("Failed to fetch info:", e);
    return null;
  }
}

async function fetchMempool() {
  try {
    const response = await fetch(
      `${CONFIG.API_BASE}/transactions/unconfirmed?limit=${CONFIG.MAX_DISPLAYED_TXS}`,
    );
    if (!response.ok) throw new Error("API error");
    return await response.json();
  } catch (e) {
    console.error("Failed to fetch mempool:", e);
    return [];
  }
}

// === UI Update Functions ===
function updateUI(data) {
  // Update state
  if (data.fullHeight !== undefined) state.fullHeight = data.fullHeight;
  if (data.headersHeight !== undefined)
    state.headersHeight = data.headersHeight;
  if (data.isSynced !== undefined) state.isSynced = data.isSynced;
  if (data.peerCount !== undefined) state.peerCount = data.peerCount;
  if (data.isMining !== undefined) state.isMining = data.isMining;
  if (data.nodeName !== undefined) state.nodeName = data.nodeName;

  if (data.mempool) {
    state.mempoolCount = data.mempool.count;
    state.mempoolSize = data.mempool.size;
    if (data.mempool.transactions) {
      state.transactions = data.mempool.transactions;
    }
  }

  // Update version if present
  if (data.appVersion) {
    elements.version.textContent = `v${data.appVersion}`;
  }

  // Update node name
  if (state.nodeName) {
    elements.nodeName.textContent = state.nodeName;
  }

  // Status indicator
  if (state.isSynced) {
    elements.statusIndicator.className = "status-indicator synced";
    elements.syncLabel.textContent = "Synchronized";
  } else if (state.headersHeight > 0) {
    elements.statusIndicator.className = "status-indicator syncing";
    const gap = state.headersHeight - state.fullHeight;
    elements.syncLabel.textContent =
      gap > 0
        ? `Syncing... (${gap.toLocaleString()} blocks behind)`
        : "Syncing...";
  } else {
    elements.statusIndicator.className = "status-indicator";
    elements.syncLabel.textContent = "Initializing...";
  }

  // Mining badge
  elements.miningBadge.style.display = state.isMining ? "flex" : "none";

  // Progress bars
  const targetHeight = Math.max(CONFIG.TARGET_HEIGHT, state.headersHeight);
  const headersPercent = (state.headersHeight / targetHeight) * 100;
  const blocksPercent = (state.fullHeight / targetHeight) * 100;

  elements.headersProgress.style.width = `${Math.min(headersPercent, 100)}%`;
  elements.headersCount.textContent = state.headersHeight.toLocaleString();

  elements.blocksProgress.style.width = `${Math.min(blocksPercent, 100)}%`;
  elements.blocksCount.textContent = state.fullHeight.toLocaleString();

  // Stats
  elements.peerCount.textContent = state.peerCount;
  elements.mempoolCount.textContent = state.mempoolCount;
  elements.mempoolSize.textContent = formatBytes(state.mempoolSize);

  // Handle recent blocks and confirmation animations
  if (data.recentBlocks !== undefined && data.isSynced) {
    const latestBlock = data.recentBlocks[0];
    const prevHeight = state.previousBlockHeight;

    // Detect new block confirmation
    if (latestBlock && latestBlock.height > prevHeight && prevHeight > 0) {
      // Animate confirmed txs DOWN into the block
      for (const txId of latestBlock.transactions) {
        if (visualizer && visualizer.transactions.has(txId)) {
          visualizer.confirmTransaction(txId);
        }
      }
    }

    state.previousBlockHeight = latestBlock?.height || 0;
    state.recentBlocks = data.recentBlocks;

    // Update block visualizer
    if (blockVisualizer) {
      blockVisualizer.updateBlocks(data.recentBlocks, data.isSynced);
    }
  } else if (!data.isSynced && blockVisualizer) {
    // Hide blocks section when not synced
    elements.blocksSection.classList.add("hidden");
  }

  // Update mempool visualization
  if (visualizer && state.transactions) {
    visualizer.updateTransactions(state.transactions);
  }
}

function updateConnectionStatus(connected, error = false) {
  state.wsConnected = connected;

  if (connected) {
    elements.connectionBanner.className = "connection-banner connected";
    elements.connectionText.textContent = "Connected";
  } else if (error) {
    elements.connectionBanner.className = "connection-banner error";
    elements.connectionText.textContent = "Connection lost - Reconnecting...";
  } else {
    elements.connectionBanner.className = "connection-banner";
    elements.connectionText.textContent = "Connecting...";
  }
}

function formatBytes(bytes) {
  if (bytes === 0) return "0 B";
  if (bytes < 1024) return bytes + " B";
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + " KB";
  return (bytes / (1024 * 1024)).toFixed(2) + " MB";
}

// === WebSocket Connection ===
let ws = null;
let wsReconnectTimer = null;

function connectWebSocket() {
  if (ws && ws.readyState === WebSocket.OPEN) return;

  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  const wsUrl = `${protocol}//${window.location.host}/panel/ws`;

  try {
    ws = new WebSocket(wsUrl);

    ws.onopen = () => {
      console.log("WebSocket connected");
      updateConnectionStatus(true);
      if (wsReconnectTimer) {
        clearTimeout(wsReconnectTimer);
        wsReconnectTimer = null;
      }
    };

    ws.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        if (data.type === "status") {
          updateUI(data);
        }
      } catch (e) {
        console.error("Failed to parse WebSocket message:", e);
      }
    };

    ws.onclose = () => {
      console.log("WebSocket disconnected");
      updateConnectionStatus(false, true);
      scheduleReconnect();
    };

    ws.onerror = (error) => {
      console.error("WebSocket error:", error);
      updateConnectionStatus(false, true);
    };
  } catch (e) {
    console.error("Failed to create WebSocket:", e);
    updateConnectionStatus(false, true);
    scheduleReconnect();
  }
}

function scheduleReconnect() {
  if (wsReconnectTimer) return;
  wsReconnectTimer = setTimeout(() => {
    wsReconnectTimer = null;
    connectWebSocket();
  }, CONFIG.WS_RECONNECT_DELAY);
}

// === Polling Fallback ===
let pollTimer = null;

async function pollUpdates() {
  // Only poll if WebSocket is not connected
  if (state.wsConnected) {
    schedulePoll();
    return;
  }

  const info = await fetchInfo();
  if (info) {
    updateUI({
      fullHeight: info.fullHeight,
      headersHeight: info.headersHeight,
      isSynced: info.isSynced,
      peerCount: info.peerCount,
      isMining: info.isMining,
      appVersion: info.appVersion,
      nodeName: info.name,
      mempool: {
        count: info.unconfirmedCount,
        size: 0, // Not available from /info
      },
    });

    // Fetch mempool transactions separately
    const txs = await fetchMempool();
    if (txs && txs.length > 0) {
      state.transactions = txs;
      if (visualizer) {
        visualizer.updateTransactions(txs);
      }
    }

    // If we got data, mark as connected (polling mode)
    if (!state.wsConnected) {
      elements.connectionBanner.className = "connection-banner connected";
      elements.connectionText.textContent = "Connected (polling)";
    }
  }

  schedulePoll();
}

function schedulePoll() {
  if (pollTimer) clearTimeout(pollTimer);
  pollTimer = setTimeout(pollUpdates, CONFIG.POLL_INTERVAL);
}

// === Initialize ===
document.addEventListener("DOMContentLoaded", () => {
  console.log("Ergo Node Panel initializing...");

  // Initialize mempool visualizer
  visualizer = new MempoolVisualizer(elements.canvas);
  visualizer.start();

  // Initialize block visualizer
  blockVisualizer = new BlockVisualizer("blocks-container");

  // Start WebSocket connection
  connectWebSocket();

  // Start polling as fallback
  pollUpdates();

  // Initial UI state
  updateConnectionStatus(false);
});

// === Cleanup on page unload ===
window.addEventListener("beforeunload", () => {
  if (ws) ws.close();
  if (visualizer) visualizer.stop();
});
