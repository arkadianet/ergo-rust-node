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
    API_BASE: '',
    WS_RECONNECT_DELAY: 3000,
    POLL_INTERVAL: 3000,
    TARGET_HEIGHT: 1500000, // Approximate mainnet height for progress calculation
    MAX_DISPLAYED_TXS: 100,
    PHYSICS: {
        GRAVITY: 0.25,
        FRICTION: 0.7,
        BOUNCE: 0.4,
        SETTLE_THRESHOLD: 0.3
    }
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
    nodeName: '',
    wsConnected: false
};

// === DOM Elements ===
const elements = {
    version: document.getElementById('version'),
    nodeName: document.getElementById('node-name'),
    connectionBanner: document.getElementById('connection-banner'),
    connectionText: document.getElementById('connection-text'),
    statusIndicator: document.getElementById('status-indicator'),
    syncLabel: document.getElementById('sync-label'),
    miningBadge: document.getElementById('mining-badge'),
    headersProgress: document.getElementById('headers-progress'),
    headersCount: document.getElementById('headers-count'),
    blocksProgress: document.getElementById('blocks-progress'),
    blocksCount: document.getElementById('blocks-count'),
    peerCount: document.getElementById('peer-count'),
    mempoolCount: document.getElementById('mempool-count'),
    mempoolSize: document.getElementById('mempool-size'),
    mempoolSection: document.getElementById('mempool-section'),
    canvas: document.getElementById('mempool-canvas'),
    emptyMempool: document.getElementById('empty-mempool')
};

// === Mempool Visualizer ===
class MempoolVisualizer {
    constructor(canvas) {
        this.canvas = canvas;
        this.ctx = canvas.getContext('2d');
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
        this.canvas.style.width = container.clientWidth + 'px';
        this.canvas.style.height = container.clientHeight + 'px';

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

        // Calculate visual properties
        const baseSize = Math.sqrt(tx.size) * 1.5;
        const size = Math.max(Math.min(baseSize, 50), 12);
        const feeRate = tx.size > 0 ? tx.fee / tx.size : 0;

        const txObj = {
            id: tx.id,
            x: Math.random() * (this.width - size),
            y: -size - Math.random() * 50,
            width: size,
            height: size * 0.7,
            vx: (Math.random() - 0.5) * 2,
            vy: 0,
            rotation: (Math.random() - 0.5) * 0.3,
            vr: (Math.random() - 0.5) * 0.02,
            color: this.feeToColor(feeRate),
            settled: false,
            removing: false,
            alpha: 1,
            fee: tx.fee,
            size: tx.size
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
        this.ctx.fillStyle = '#0f0f1a';
        this.ctx.fillRect(0, 0, this.width, this.height);

        // Draw ground line
        this.ctx.strokeStyle = 'rgba(255, 255, 255, 0.05)';
        this.ctx.lineWidth = 1;
        this.ctx.beginPath();
        this.ctx.moveTo(0, this.height - 5);
        this.ctx.lineTo(this.width, this.height - 5);
        this.ctx.stroke();

        // Sort transactions by y position for proper rendering
        const sortedTxs = Array.from(this.transactions.values())
            .sort((a, b) => a.y - b.y);

        for (const tx of sortedTxs) {
            this.updateTransaction(tx);
            this.drawTransaction(tx);
        }

        requestAnimationFrame(() => this.animate());
    }

    updateTransaction(tx) {
        const { GRAVITY, FRICTION, BOUNCE, SETTLE_THRESHOLD } = CONFIG.PHYSICS;

        if (tx.removing) {
            // Fly up and fade out
            tx.vy -= 0.5;
            tx.y += tx.vy;
            tx.rotation += tx.vr;
            tx.alpha = Math.max(0, tx.alpha - 0.03);
            return;
        }

        if (tx.settled) return;

        // Apply gravity
        tx.vy += GRAVITY;

        // Apply velocity
        tx.x += tx.vx;
        tx.y += tx.vy;
        tx.rotation += tx.vr;

        // Floor collision
        const floor = this.height - tx.height - 5;
        if (tx.y > floor) {
            tx.y = floor;
            tx.vy *= -BOUNCE;
            tx.vx *= FRICTION;
            tx.vr *= FRICTION;

            // Check if settled
            if (Math.abs(tx.vy) < SETTLE_THRESHOLD) {
                tx.settled = true;
                tx.vy = 0;
                tx.vr = 0;
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
            if (other === tx || other.removing) continue;
            if (!other.settled && other.y < tx.y) continue;

            if (this.checkCollision(tx, other)) {
                // Push up
                const overlap = (tx.y + tx.height) - other.y;
                if (overlap > 0 && tx.vy > 0) {
                    tx.y = other.y - tx.height;
                    tx.vy *= -BOUNCE * 0.5;

                    if (Math.abs(tx.vy) < SETTLE_THRESHOLD) {
                        tx.settled = true;
                        tx.vy = 0;
                    }
                }
            }
        }
    }

    checkCollision(a, b) {
        return a.x < b.x + b.width &&
               a.x + a.width > b.x &&
               a.y < b.y + b.height &&
               a.y + a.height > b.y;
    }

    drawTransaction(tx) {
        this.ctx.save();
        this.ctx.globalAlpha = tx.alpha;

        // Transform for rotation
        const cx = tx.x + tx.width / 2;
        const cy = tx.y + tx.height / 2;
        this.ctx.translate(cx, cy);
        this.ctx.rotate(tx.rotation);

        // Draw rounded rectangle
        const x = -tx.width / 2;
        const y = -tx.height / 2;
        const radius = 3;

        this.ctx.beginPath();
        this.ctx.moveTo(x + radius, y);
        this.ctx.lineTo(x + tx.width - radius, y);
        this.ctx.quadraticCurveTo(x + tx.width, y, x + tx.width, y + radius);
        this.ctx.lineTo(x + tx.width, y + tx.height - radius);
        this.ctx.quadraticCurveTo(x + tx.width, y + tx.height, x + tx.width - radius, y + tx.height);
        this.ctx.lineTo(x + radius, y + tx.height);
        this.ctx.quadraticCurveTo(x, y + tx.height, x, y + tx.height - radius);
        this.ctx.lineTo(x, y + radius);
        this.ctx.quadraticCurveTo(x, y, x + radius, y);
        this.ctx.closePath();

        // Fill with gradient
        const gradient = this.ctx.createLinearGradient(x, y, x, y + tx.height);
        gradient.addColorStop(0, tx.color);
        gradient.addColorStop(1, this.darkenColor(tx.color, 20));
        this.ctx.fillStyle = gradient;
        this.ctx.fill();

        // Border
        this.ctx.strokeStyle = 'rgba(255, 255, 255, 0.2)';
        this.ctx.lineWidth = 1;
        this.ctx.stroke();

        // Highlight on removing (confirmed)
        if (tx.removing) {
            this.ctx.fillStyle = `rgba(74, 222, 128, ${tx.alpha * 0.5})`;
            this.ctx.fill();
        }

        this.ctx.restore();
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
        const newIds = new Set(newTxs.map(t => t.id));

        // Add new transactions
        for (const tx of newTxs) {
            if (!currentIds.has(tx.id)) {
                this.addTransaction(tx);
            }
        }

        // Remove confirmed transactions (not in new list)
        for (const id of currentIds) {
            if (!newIds.has(id)) {
                this.removeTransaction(id);
            }
        }

        // Update empty state
        elements.emptyMempool.classList.toggle('hidden', this.transactions.size > 0);
    }

    clear() {
        this.transactions.clear();
    }
}

// === Global visualizer instance ===
let visualizer = null;

// === API Functions ===
async function fetchInfo() {
    try {
        const response = await fetch(`${CONFIG.API_BASE}/info`);
        if (!response.ok) throw new Error('API error');
        return await response.json();
    } catch (e) {
        console.error('Failed to fetch info:', e);
        return null;
    }
}

async function fetchMempool() {
    try {
        const response = await fetch(`${CONFIG.API_BASE}/transactions/unconfirmed?limit=${CONFIG.MAX_DISPLAYED_TXS}`);
        if (!response.ok) throw new Error('API error');
        return await response.json();
    } catch (e) {
        console.error('Failed to fetch mempool:', e);
        return [];
    }
}

// === UI Update Functions ===
function updateUI(data) {
    // Update state
    if (data.fullHeight !== undefined) state.fullHeight = data.fullHeight;
    if (data.headersHeight !== undefined) state.headersHeight = data.headersHeight;
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
        elements.statusIndicator.className = 'status-indicator synced';
        elements.syncLabel.textContent = 'Synchronized';
    } else if (state.headersHeight > 0) {
        elements.statusIndicator.className = 'status-indicator syncing';
        const gap = state.headersHeight - state.fullHeight;
        elements.syncLabel.textContent = gap > 0
            ? `Syncing... (${gap.toLocaleString()} blocks behind)`
            : 'Syncing...';
    } else {
        elements.statusIndicator.className = 'status-indicator';
        elements.syncLabel.textContent = 'Initializing...';
    }

    // Mining badge
    elements.miningBadge.style.display = state.isMining ? 'flex' : 'none';

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

    // Update mempool visualization
    if (visualizer && state.transactions) {
        visualizer.updateTransactions(state.transactions);
    }
}

function updateConnectionStatus(connected, error = false) {
    state.wsConnected = connected;

    if (connected) {
        elements.connectionBanner.className = 'connection-banner connected';
        elements.connectionText.textContent = 'Connected';
    } else if (error) {
        elements.connectionBanner.className = 'connection-banner error';
        elements.connectionText.textContent = 'Connection lost - Reconnecting...';
    } else {
        elements.connectionBanner.className = 'connection-banner';
        elements.connectionText.textContent = 'Connecting...';
    }
}

function formatBytes(bytes) {
    if (bytes === 0) return '0 B';
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
    return (bytes / (1024 * 1024)).toFixed(2) + ' MB';
}

// === WebSocket Connection ===
let ws = null;
let wsReconnectTimer = null;

function connectWebSocket() {
    if (ws && ws.readyState === WebSocket.OPEN) return;

    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const wsUrl = `${protocol}//${window.location.host}/panel/ws`;

    try {
        ws = new WebSocket(wsUrl);

        ws.onopen = () => {
            console.log('WebSocket connected');
            updateConnectionStatus(true);
            if (wsReconnectTimer) {
                clearTimeout(wsReconnectTimer);
                wsReconnectTimer = null;
            }
        };

        ws.onmessage = (event) => {
            try {
                const data = JSON.parse(event.data);
                if (data.type === 'status') {
                    updateUI(data);
                }
            } catch (e) {
                console.error('Failed to parse WebSocket message:', e);
            }
        };

        ws.onclose = () => {
            console.log('WebSocket disconnected');
            updateConnectionStatus(false, true);
            scheduleReconnect();
        };

        ws.onerror = (error) => {
            console.error('WebSocket error:', error);
            updateConnectionStatus(false, true);
        };
    } catch (e) {
        console.error('Failed to create WebSocket:', e);
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
                size: 0 // Not available from /info
            }
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
            elements.connectionBanner.className = 'connection-banner connected';
            elements.connectionText.textContent = 'Connected (polling)';
        }
    }

    schedulePoll();
}

function schedulePoll() {
    if (pollTimer) clearTimeout(pollTimer);
    pollTimer = setTimeout(pollUpdates, CONFIG.POLL_INTERVAL);
}

// === Initialize ===
document.addEventListener('DOMContentLoaded', () => {
    console.log('Ergo Node Panel initializing...');

    // Initialize mempool visualizer
    visualizer = new MempoolVisualizer(elements.canvas);
    visualizer.start();

    // Start WebSocket connection
    connectWebSocket();

    // Start polling as fallback
    pollUpdates();

    // Initial UI state
    updateConnectionStatus(false);
});

// === Cleanup on page unload ===
window.addEventListener('beforeunload', () => {
    if (ws) ws.close();
    if (visualizer) visualizer.stop();
});
